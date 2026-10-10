//! What caused what.
//!
//! Every observable point - an action from the UI, a message sent to an actor,
//! its handling, background work and its result, a publication and each of its
//! deliveries, a reducer update, a navigation - is a [`Record`] with an id and
//! the id of the point that caused it. Following `parent` from any record
//! gives its provenance; following it the other way gives everything it set
//! off.
//!
//! The cause is carried implicitly on the thread ([`current`]) and explicitly
//! wherever work crosses a queue or a thread: an actor's mailbox, a background
//! task, a hop back onto the UI thread. [`Cause`] is `Copy + Send` for that.
//!
//! Records go to the observers on the thread that produced them (devtools),
//! and to `tracing` as one event each: the target names the kind of point,
//! `guinea::send`, and the fields carry `id`, `parent` and what the point
//! holds. `guinea::tick=off` silences one kind. [`json`] writes them, and the
//! application's own events with the point they happened under, as JSON
//! lines.

mod json;
mod point;
mod sink;

pub use json::{Json, json};
pub use point::{Bus, Point, StoreOp};
pub use sink::{
    ELSEWHERE_LIMIT, Elsewhere, Observer, is_observed, is_observed_anywhere, is_point_target,
    is_recorded_anywhere, observe, stop_observing, take_elsewhere, thread_id,
};

use std::cell::Cell;
use std::num::NonZeroU64;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// One observed point, named for being the cause of what follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cause(NonZeroU64);

impl Cause {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Cause(NonZeroU64::new(NEXT.fetch_add(1, Ordering::Relaxed)).expect("ids start at 1"))
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: Cause,
    pub parent: Option<Cause>,
    /// Since the process first traced anything.
    pub at: Duration,
    pub point: Point,
}

/// What an observer is told.
#[derive(Clone, Debug, PartialEq)]
pub enum Trace {
    /// A point that stays current until [`Trace::End`]: what happens inside it
    /// is caused by it.
    Begin(Record),
    End { id: Cause, took: Duration },
    /// A point with no extent: a send, a push, a spawn.
    Mark(Record),
}

fn start() -> &'static (Instant, SystemTime) {
    static START: OnceLock<(Instant, SystemTime)> = OnceLock::new();
    START.get_or_init(|| (Instant::now(), SystemTime::now()))
}

/// Time since the moment [`Record::at`] counts from, on the same monotonic
/// clock.
pub fn now() -> Duration {
    start().0.elapsed()
}

/// The wall clock at the moment [`Record::at`] counts from.
pub fn started_at() -> SystemTime {
    start().1
}

thread_local! {
    static CURRENT: Cell<Option<Cause>> = const { Cell::new(None) };
}

/// What is happening on this thread right now, if anything is.
pub fn current() -> Option<Cause> {
    CURRENT.with(Cell::get)
}

thread_local! {
    /// What recording has cost since the point that is current now opened.
    ///
    /// Observing is not free - a record is built, given to devtools, written
    /// through `tracing` - and it all happens inside whatever is being
    /// measured. Charged to the observer instead, so a duration means the
    /// same whether or not anyone is watching.
    static WATCHING: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// Runs `recording` and charges what it took to the observer rather than to
/// the point that is open.
fn observed<R>(recording: impl FnOnce() -> R) -> R {
    let started = Instant::now();
    let done = recording();

    WATCHING.with(|watching| watching.set(watching.get() + started.elapsed()));

    done
}

/// Records a point caused by whatever is current, without making it current.
pub fn mark(point: impl FnOnce() -> Point) -> Cause {
    mark_under(current(), point)
}

/// Records a point caused by `parent`, for work that crossed a queue or a
/// thread and brought its cause along.
pub fn mark_under(parent: Option<Cause>, point: impl FnOnce() -> Point) -> Cause {
    let id = Cause::next();
    if sink::wanted() {
        observed(|| {
            sink::emit(Trace::Mark(Record {
                id,
                parent,
                at: now(),
                point: point(),
            }));
        });
    }
    id
}

/// Records a point caused by whatever is current, and makes it current until
/// the guard drops.
pub fn enter(point: impl FnOnce() -> Point) -> Entered {
    enter_under(current(), point)
}

/// [`enter`], with the cause given rather than taken from the thread.
pub fn enter_under(parent: Option<Cause>, point: impl FnOnce() -> Point) -> Entered {
    let id = Cause::next();
    let started = now();
    if sink::wanted() {
        observed(|| {
            sink::emit(Trace::Begin(Record {
                id,
                parent,
                at: started,
                point: point(),
            }));
        });
    }
    let previous = CURRENT.with(|current| current.replace(Some(id)));

    Entered {
        id,
        previous,
        started,
        // What observing had cost before this point opened. Whatever is
        // added to it while the point is open was spent watching it, not
        // doing it.
        watched: WATCHING.with(Cell::get),
    }
}

/// An id for a point recorded later, by [`begin_as`]. For a point that is not
/// entered once and left: what happens under it may be recorded before it is,
/// on another thread, and needs its id to say so.
pub fn reserve() -> Cause {
    Cause::next()
}

/// Records `point` as open, under `parent` and with an id [`reserve`] gave,
/// without making it current: it is current wherever it is [`resume`]d, and
/// closes at [`end`].
pub fn begin_as(id: Cause, parent: Option<Cause>, point: impl FnOnce() -> Point) {
    if sink::wanted() {
        observed(|| {
            sink::emit(Trace::Begin(Record {
                id,
                parent,
                at: now(),
                point: point(),
            }));
        });
    }
}

/// Closes a point [`begin_as`] opened, saying what it took.
pub fn end(id: Cause, took: Duration) {
    if sink::wanted() {
        observed(|| sink::emit(Trace::End { id, took }));
    }
}

/// Makes `cause` current until the guard drops, without recording anything:
/// for code that continues a point recorded elsewhere.
pub fn resume(cause: Option<Cause>) -> Resumed {
    Resumed {
        previous: CURRENT.with(|current| current.replace(cause)),
    }
}

/// Runs `future` with `cause` current on whichever thread polls it.
///
/// A guard held across `.await` would stay on the thread the task started on;
/// this sets the cause for each poll instead.
pub fn within<F: Future>(cause: Option<Cause>, future: F) -> Within<F> {
    Within {
        cause,
        future: Box::pin(future),
    }
}

pub struct Within<F> {
    cause: Option<Cause>,
    future: std::pin::Pin<Box<F>>,
}

impl<F: Future> Future for Within<F> {
    type Output = F::Output;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<F::Output> {
        let _resumed = resume(self.cause);
        self.future.as_mut().poll(cx)
    }
}

#[must_use = "the point stops being current when this is dropped"]
pub struct Entered {
    id: Cause,
    previous: Option<Cause>,
    started: Duration,
    /// What observing had cost by the time this point opened.
    watched: Duration,
}

impl Entered {
    pub fn id(&self) -> Cause {
        self.id
    }
}

impl Drop for Entered {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.previous));

        if sink::wanted() {
            // Everything observing cost while this point was open comes off
            // what the point is said to have taken. It stays on the running
            // total, so the point above this one discounts it too - it was
            // open for all of it as well.
            let watching = WATCHING.with(Cell::get).saturating_sub(self.watched);
            let took = now().saturating_sub(self.started).saturating_sub(watching);

            observed(|| sink::emit(Trace::End { id: self.id, took }));
        }
    }
}

#[must_use = "the cause stops being current when this is dropped"]
pub struct Resumed {
    previous: Option<Cause>,
}

impl Drop for Resumed {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn collect() -> Rc<RefCell<Vec<Trace>>> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        observe(move |trace| sink.borrow_mut().push(trace.clone()));
        seen
    }

    /// Every trace, as its kind and its id, in the order it was seen: a test
    /// compares the whole of it, so a record it did not make fails it.
    fn order(seen: &[Trace]) -> Vec<(&'static str, Cause)> {
        seen.iter()
            .map(|trace| match trace {
                Trace::Begin(record) => ("begin", record.id),
                Trace::Mark(record) => ("mark", record.id),
                Trace::End { id, .. } => ("end", *id),
            })
            .collect()
    }

    fn parent_of(seen: &[Trace], id: Cause) -> Option<Cause> {
        seen.iter().find_map(|trace| match trace {
            Trace::Begin(record) | Trace::Mark(record) if record.id == id => Some(record.parent),
            _ => None,
        })?
    }

    #[test]
    fn now_is_read_on_the_clock_a_record_is_stamped_with() {
        let seen = collect();

        let before = mark(|| Point::Push { reducer: "Before" });
        let between = now();
        let after = mark(|| Point::Push { reducer: "After" });
        stop_observing();

        let seen = seen.borrow();
        assert_eq!(order(&seen), [("mark", before), ("mark", after)], "{seen:#?}");

        let at: Vec<Duration> = seen
            .iter()
            .filter_map(|trace| match trace {
                Trace::Mark(record) => Some(record.at),
                _ => None,
            })
            .collect();
        assert!(at[0] <= between && between <= at[1], "{at:?} around {between:?}");
    }

    /// Observing costs time, and it is spent inside whatever is open. Left
    /// in, a duration would say how long the work took *while watched*,
    /// which is not a number anyone wants.
    #[test]
    fn what_a_point_took_leaves_out_what_watching_it_cost() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        observe(move |trace| {
            // An observer that takes its time, so the cost is unmistakable.
            std::thread::sleep(std::time::Duration::from_millis(2));
            sink.borrow_mut().push(trace.clone());
        });

        let action = enter(|| Point::Action { message: "Save" });
        let pushes: Vec<Cause> = (0..5)
            .map(|_| mark(|| Point::Push { reducer: "Metrics" }))
            .collect();
        let action_id = action.id();
        drop(action);
        stop_observing();

        let seen = seen.borrow();
        let expected: Vec<(&str, Cause)> = std::iter::once(("begin", action_id))
            .chain(pushes.iter().map(|push| ("mark", *push)))
            .chain(std::iter::once(("end", action_id)))
            .collect();
        assert_eq!(order(&seen), expected, "{seen:#?}");

        let Some(&Trace::End { took, .. }) = seen.last() else {
            unreachable!("the order above ends with the action's end")
        };
        assert!(
            took < std::time::Duration::from_millis(5),
            "five marks at two milliseconds of observer each were charged to the action: {took:?}"
        );
    }

    #[test]
    fn what_happens_inside_a_point_is_caused_by_it() {
        let seen = collect();

        let action = enter(|| Point::Action { message: "Kill" });
        let send = mark(|| Point::Send {
            actor: "ProcessActor",
            message: "Kill",
        });
        let handled = enter_under(Some(send), || Point::Handle {
            actor: "ProcessActor",
            message: "Kill",
        });
        let publish = mark(|| Point::Publish {
            event: "ProcessKilled",
            bus: Bus::Global,
            subscribers: 2,
        });
        let handle_id = handled.id();
        let action_id = action.id();
        drop(handled);
        drop(action);
        stop_observing();

        let seen = seen.borrow();
        assert_eq!(
            order(&seen),
            [
                ("begin", action_id),
                ("mark", send),
                ("begin", handle_id),
                ("mark", publish),
                ("end", handle_id),
                ("end", action_id),
            ],
            "{seen:#?}"
        );
        assert_eq!(parent_of(&seen, send), Some(action_id));
        assert_eq!(parent_of(&seen, handle_id), Some(send));
        assert_eq!(parent_of(&seen, publish), Some(handle_id));
        assert_eq!(parent_of(&seen, action_id), None);
        assert!(matches!(seen.last(), Some(Trace::End { id, .. }) if *id == action_id));
        assert_eq!(current(), None, "everything entered has been left");
    }

    #[test]
    fn a_resumed_cause_is_the_parent_of_what_follows_and_goes_away_after() {
        let seen = collect();
        let spawn = mark(|| Point::Spawn {
            actor: "Poller",
            actor_id: 1,
            output: "Tick",
        });
        let resumed = resume(Some(spawn));
        let pushed = mark(|| Point::Push { reducer: "Metrics" });
        drop(resumed);
        let after = mark(|| Point::Push { reducer: "Metrics" });
        stop_observing();

        let seen = seen.borrow();
        assert_eq!(
            order(&seen),
            [("mark", spawn), ("mark", pushed), ("mark", after)],
            "{seen:#?}"
        );
        assert_eq!(parent_of(&seen, pushed), Some(spawn));
        assert_eq!(parent_of(&seen, after), None);
    }

    type Event = (String, Vec<(String, String)>);

    #[derive(Clone, Default)]
    struct Written(std::sync::Arc<std::sync::Mutex<Vec<Event>>>);

    struct Fields(Vec<(String, String)>);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push((field.name().to_string(), format!("{value:?}")));
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Written {
        fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);

            let target = event.metadata().target().to_string();
            self.0.lock().unwrap().push((target, fields.0));
        }
    }

    #[test]
    fn a_point_goes_to_tracing_as_its_kind_with_what_it_holds_as_fields() {
        use tracing_subscriber::layer::SubscriberExt;

        let written = Written::default();
        let subscriber = tracing_subscriber::registry().with(written.clone());

        tracing::subscriber::with_default(subscriber, || {
            let send = mark(|| Point::Send {
                actor: "ProcessActor",
                message: "Kill",
            });
            mark_under(Some(send), || Point::Log {
                level: tracing::Level::INFO,
                target: "app",
                file: None,
                line: None,
                module: None,
                text: "already written by whoever logged it".into(),
            });
        });

        let written = written.0.lock().unwrap();
        let fields: Vec<(&str, &str)> = written[0]
            .1
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();

        assert_eq!(written.len(), 1, "a log point is not written back: {written:?}");
        assert_eq!(written[0].0, "guinea::send");
        assert!(fields.contains(&("actor", "ProcessActor")), "{fields:?}");
        assert!(fields.contains(&("msg", "Kill")), "{fields:?}");
        assert!(!fields.iter().any(|(name, _)| *name == "parent"), "no cause, no parent: {fields:?}");
        assert!(!fields.iter().any(|(name, _)| *name == "message"), "no prose: {fields:?}");
    }
}
