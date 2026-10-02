//! Timers, owned by the context that set them up.
//!
//! An application's timers live as long as the application; a feature's, as
//! long as its scope. There is no timer without an owner: dropping what the
//! context holds stops it. Each remembers where it was set up, which is how
//! devtools tell one from another, and ticks on the UI thread.
//!
//! They wait on `tokio::time`, through the same executor as background work,
//! so they need the runtime `spawn_bg` needs - and under a test harness they
//! wait on the test's clock and tick when it is advanced.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::Location;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use guinea_core::actor::invoke_on_ui;
use guinea_core::devtools::{self, Change};
use guinea_core::scope::Awake;

/// How long between ticks.
pub enum Period {
    Fixed(Duration),
    /// Asked again before every tick: for a period the user can change.
    Varying(Box<dyn Fn() -> Duration>),
    /// See [`Period::follows`].
    Following {
        now: Box<dyn Fn() -> Option<Duration>>,
        watch: Box<dyn Fn(Box<dyn Fn() + Send + Sync>) -> Box<dyn Any>>,
    },
}

/// A value that changes and says so: what a period [follows](Period::follows).
///
/// A store's reactive value gets it from the crate that knows both - for
/// amethystate's `Field` and `ReactiveCell`, `.changing()` from
/// `amethystate-guinea` - and anything else that can be read and watched
/// implements it by hand.
pub trait Changing: 'static {
    type Value;

    /// What it is now, or `None` while there is nothing to read.
    fn now(&self) -> Option<Self::Value>;

    /// Calls `changed` every time it changes, from whichever thread the
    /// change happens on, until what this returns is dropped.
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any>;
}

impl Period {
    pub fn varying(period: impl Fn() -> Duration + 'static) -> Self {
        Period::Varying(Box::new(period))
    }

    /// As long as `source` says, through `period`: a change cuts the wait
    /// under way short and the next tick comes a new period after it, the
    /// way [`Timer::period`] does. While `source` has nothing to read, the
    /// timer does not tick.
    ///
    /// ```ignore
    /// use amethystate_guinea::IntoChanging;
    ///
    /// cx.every(Period::follows(settings.ping_interval_ms().changing(), Duration::from_millis), &agent, || Ping);
    /// ```
    pub fn follows<C: Changing>(source: C, period: impl Fn(C::Value) -> Duration + 'static) -> Self {
        let source = Rc::new(source);
        let watched = source.clone();

        Period::Following {
            now: Box::new(move || source.now().map(&period)),
            watch: Box::new(move |changed| watched.watch(changed)),
        }
    }

    /// The wait before the next tick, or `None` while there is nothing to
    /// wait by.
    fn next(&self) -> Option<Duration> {
        match self {
            Period::Fixed(period) => Some(*period),
            Period::Varying(period) => Some(period()),
            Period::Following { now, .. } => now(),
        }
    }

    /// Starts watching what it follows, for timer `id`: a change re-arms it
    /// on the UI thread, wherever the change happened.
    fn watch(&self, id: u64) -> Option<Box<dyn Any>> {
        match self {
            Period::Following { watch, .. } => {
                Some(watch(Box::new(move || invoke_on_ui(move || follow(id)))))
            }
            _ => None,
        }
    }
}

impl From<Duration> for Period {
    fn from(period: Duration) -> Self {
        Period::Fixed(period)
    }
}

/// What is known about a running timer.
#[derive(Clone, Debug)]
pub struct TimerInfo {
    pub id: u64,
    pub name: Option<&'static str>,
    /// Where it was set up.
    pub place: &'static Location<'static>,
    /// The feature that set it up.
    pub feature: Option<&'static str>,
    /// The scope that owns it, as `Scope::key` names it; `None` for the
    /// application.
    pub scope: Option<usize>,
    /// The period of the last tick, or of the first one to come.
    pub period: Duration,
}

struct Entry {
    info: TimerInfo,
    period: Period,
    /// Which chain of wake-ups is the live one. Changing the period starts a
    /// new chain, and whatever the old one had already queued is dropped
    /// when it comes due.
    generation: u64,
    active: Option<Box<dyn Fn() -> bool>>,
    /// The owning scope's, when a scope owns it: asleep, it skips its ticks.
    awake: Option<Awake>,
    /// What keeps the value a [`Period::Following`] follows watched.
    watching: Option<Box<dyn Any>>,
    run: Box<dyn FnMut()>,
    traced: bool,
}

thread_local! {
    static RUNNING: RefCell<HashMap<u64, Weak<RefCell<Entry>>>> = RefCell::new(HashMap::new());
    static PENDING: RefCell<HashMap<u64, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PENDING: AtomicU64 = AtomicU64::new(1);

/// What keeps a timer running; the context that set it up holds it.
pub struct Ticking {
    entry: Rc<RefCell<Entry>>,
}

impl Drop for Ticking {
    fn drop(&mut self) {
        let (id, traced) = {
            let entry = self.entry.borrow();
            (entry.info.id, entry.traced)
        };
        RUNNING.with(|running| running.borrow_mut().remove(&id));

        if traced {
            devtools::changed(|| Change::TimerStopped { id });
        }
    }
}

impl guinea_core::scope::Teardown for Ticking {
    fn teardown(self) {
        drop(self);
    }
}

/// A timer that was set up, for saying more about it.
///
/// Holding it does not keep the timer running, and dropping it does not stop
/// it: that is up to the context.
#[derive(Clone)]
pub struct Timer {
    entry: Weak<RefCell<Entry>>,
}

impl Timer {
    fn change(self, change: impl FnOnce(&mut Entry)) -> Self {
        if let Some(entry) = self.entry.upgrade() {
            change(&mut entry.borrow_mut());
        }
        self
    }

    /// What devtools call it, instead of where it was set up.
    pub fn named(self, name: &'static str) -> Self {
        self.change(|entry| entry.info.name = Some(name))
    }

    /// Ticks only while `active` says so; the period runs on regardless.
    pub fn when(self, active: impl Fn() -> bool + 'static) -> Self {
        self.change(|entry| entry.active = Some(Box::new(active)))
    }

    /// Changes how often it ticks, from now.
    ///
    /// The wait already under way is cut short rather than waited out, which
    /// is the difference from a [`Period::Varying`] that answers differently
    /// the next time it is asked: an hourly timer told to tick every second
    /// does so within the second, not within the hour.
    pub fn period(self, period: impl Into<Period>) -> Self {
        let Some(entry) = self.entry.upgrade() else {
            return self;
        };

        let id = entry.borrow().info.id;
        let period = period.into();
        let watching = period.watch(id);

        let (generation, next, unwatched) = {
            let mut entry = entry.borrow_mut();
            entry.period = period;
            let unwatched = std::mem::replace(&mut entry.watching, watching);
            entry.generation += 1;

            let next = entry.period.next();
            if let Some(next) = next {
                entry.info.period = next;
            }

            (entry.generation, next, unwatched)
        };
        drop(unwatched);

        wake_after(id, generation, next);
        self
    }

    /// Leaves no trace and does not show up in devtools: for tooling that
    /// watches the application and must not be seen in what it watches.
    pub fn untraced(self) -> Self {
        let Some(entry) = self.entry.upgrade() else {
            return self;
        };

        let (id, was_traced) = {
            let mut entry = entry.borrow_mut();
            (entry.info.id, std::mem::replace(&mut entry.traced, false))
        };
        if was_traced {
            devtools::changed(|| Change::TimerStopped { id });
        }

        self
    }

    pub fn id(&self) -> Option<u64> {
        self.entry.upgrade().map(|entry| entry.borrow().info.id)
    }
}

/// Starts a timer: what keeps it running, for the context to hold, and the
/// timer itself, for the caller to name.
pub(crate) fn start(
    place: &'static Location<'static>,
    feature: Option<&'static str>,
    owner: Option<(usize, Awake)>,
    period: Period,
    run: impl FnMut() + 'static,
) -> (Ticking, Timer) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let first = period.next();
    let watching = period.watch(id);
    let (scope, awake) = owner.unzip();

    let entry = Rc::new(RefCell::new(Entry {
        info: TimerInfo {
            id,
            name: None,
            place,
            feature,
            scope,
            period: first.unwrap_or_default(),
        },
        period,
        generation: 0,
        active: None,
        awake,
        watching,
        run: Box::new(run),
        traced: true,
    }));

    RUNNING.with(|running| running.borrow_mut().insert(id, Rc::downgrade(&entry)));
    devtools::changed(|| Change::TimerStarted { id });
    wake_after(id, 0, first);

    let timer = Timer {
        entry: Rc::downgrade(&entry),
    };
    (Ticking { entry }, timer)
}

/// The timers running on this thread that devtools may see, oldest first.
pub fn running() -> Vec<TimerInfo> {
    let mut timers: Vec<TimerInfo> = RUNNING.with(|running| {
        running
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .filter(|entry| entry.borrow().traced)
            .map(|entry| entry.borrow().info.clone())
            .collect()
    });

    timers.sort_by_key(|timer| timer.id);
    timers
}

fn tick(id: u64, generation: u64) {
    let entry = RUNNING.with(|running| running.borrow().get(&id).and_then(Weak::upgrade));
    let Some(entry) = entry else {
        return;
    };

    // Left over from a period that has since changed: the chain it belonged
    // to ended when the new one was armed.
    if entry.borrow().generation != generation {
        return;
    }

    let (active, traced) = {
        let entry = entry.borrow();
        let active = entry.active.as_ref().is_none_or(|active| active())
            && entry.awake.as_ref().is_none_or(Awake::now);
        (active, entry.traced)
    };

    if active {
        let mut run = std::mem::replace(&mut entry.borrow_mut().run, Box::new(|| {}));

        {
            let _tick = traced
                .then(|| guinea_core::trace::enter(|| guinea_core::trace::Point::Tick { timer: id }));
            run();
        }

        entry.borrow_mut().run = run;
    }

    let next = {
        let mut entry = entry.borrow_mut();
        let next = entry.period.next();
        if let Some(next) = next {
            entry.info.period = next;
        }

        next
    };

    // With the generation this tick came from, not the one the entry holds
    // now: a `Timer::period` from inside `run` has already armed its own
    // chain, and this one is over.
    wake_after(id, generation, next);
}

/// What a [`Period::Following`] does when its value changes: starts a new
/// chain from now, as [`Timer::period`] does, and ends the one under way.
fn follow(id: u64) {
    let entry = RUNNING.with(|running| running.borrow().get(&id).and_then(Weak::upgrade));
    let Some(entry) = entry else {
        return;
    };

    let (generation, next) = {
        let mut entry = entry.borrow_mut();
        entry.generation += 1;

        let next = entry.period.next();
        if let Some(next) = next {
            entry.info.period = next;
        }

        (entry.generation, next)
    };

    wake_after(id, generation, next);
}

/// Arms one tick: a task that waits `after` on the clock guinea's work runs by
/// and hands the tick to the UI thread - tokio's in an application, the test's
/// own under a harness. With nothing to wait by, the chain ends here.
fn wake_after(id: u64, generation: u64, after: Option<Duration>) {
    let Some(after) = after else {
        return;
    };

    guinea_core::executor::spawn(async move {
        tokio::time::sleep(after).await;
        invoke_on_ui(move || tick(id, generation));
    });
}

/// Runs `run` once on the UI thread, `delay` from now: for gathering what
/// happens in a burst into one go.
///
/// Not a timer - nothing lists it, nothing traces it - and nothing holds it:
/// `run` should hold weakly whatever may be gone by the time it runs.
pub fn after(delay: Duration, run: impl FnOnce() + 'static) {
    let id = NEXT_PENDING.fetch_add(1, Ordering::Relaxed);
    PENDING.with(|pending| pending.borrow_mut().insert(id, Box::new(run)));

    guinea_core::executor::spawn(async move {
        tokio::time::sleep(delay).await;
        invoke_on_ui(move || {
            let run = PENDING.with(|pending| pending.borrow_mut().remove(&id));
            if let Some(run) = run {
                run();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_core::executor::{Installed, install};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    /// The test's own clock: timers wait on it, and it moves only when told.
    fn clock() -> Installed {
        install(0)
    }

    fn wait(clock: &Installed, ms: u64) {
        clock.advance(Duration::from_millis(ms));
    }

    fn count(counter: &Arc<AtomicUsize>) -> usize {
        counter.load(Ordering::SeqCst)
    }

    fn counting(period: u64) -> (Ticking, Timer, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        let (ticking, timer) = start(
            Location::caller(),
            None,
            None,
            Duration::from_millis(period).into(),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );

        (ticking, timer, counter)
    }

    fn watched(run: impl FnOnce()) -> Vec<Change> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        devtools::watch(move |change| sink.borrow_mut().push(change.clone()));

        run();
        devtools::stop_watching();

        seen.take()
    }

    #[test]
    fn devtools_hear_when_a_timer_starts_and_when_it_stops() {
        let _clock = clock();
        let mut id = None;

        let seen = watched(|| {
            let (ticking, timer, _) = counting(30);
            id = timer.id();
            drop(ticking);
        });

        let id = id.expect("running");
        assert_eq!(seen, [Change::TimerStarted { id }, Change::TimerStopped { id }]);
    }

    #[test]
    fn an_untraced_timer_leaves_devtools_at_once_and_stops_unheard() {
        let _clock = clock();
        let mut id = None;

        let seen = watched(|| {
            let (ticking, timer, _) = counting(30);
            id = timer.id();
            let _timer = timer.untraced();
            drop(ticking);
        });

        let id = id.expect("running");
        assert_eq!(seen, [Change::TimerStarted { id }, Change::TimerStopped { id }]);
    }

    #[test]
    fn after_runs_once_when_its_delay_is_up() {
        let clock = clock();
        let ran = Rc::new(std::cell::Cell::new(0));

        let counter = ran.clone();
        after(Duration::from_millis(16), move || counter.set(counter.get() + 1));

        wait(&clock, 15);
        assert_eq!(ran.get(), 0, "ran before its delay was up");

        wait(&clock, 1);
        assert_eq!(ran.get(), 1);

        wait(&clock, 100);
        assert_eq!(ran.get(), 1, "ran again");
    }

    #[test]
    fn after_is_no_timer_devtools_hear_of() {
        let clock = clock();

        let seen = watched(|| {
            after(Duration::from_millis(16), || {});
            wait(&clock, 16);
        });

        assert_eq!(seen, []);
    }

    #[test]
    fn a_timer_ticks_every_period() {
        let clock = clock();
        let (_ticking, _timer, counter) = counting(30);

        wait(&clock, 29);
        assert_eq!(count(&counter), 0, "ticked before its period was up");

        wait(&clock, 1);
        assert_eq!(count(&counter), 1);

        wait(&clock, 30);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_timer_stops_with_what_keeps_it() {
        let clock = clock();
        let (ticking, _timer, counter) = counting(30);

        wait(&clock, 30);
        assert_eq!(count(&counter), 1);
        drop(ticking);

        wait(&clock, 90);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn a_timer_dropped_before_its_first_tick_never_ticks() {
        let clock = clock();
        let (ticking, _timer, counter) = counting(200);

        drop(ticking);

        wait(&clock, 250);
        assert_eq!(count(&counter), 0);
    }

    #[test]
    fn changing_the_period_cuts_the_wait_short_and_ends_the_old_chain() {
        let clock = clock();
        let (_ticking, timer, counter) = counting(60_000);

        let timer = timer.period(Duration::from_millis(20));
        wait(&clock, 20);
        assert_eq!(count(&counter), 1, "a minute's wait was waited out");

        let _timer = timer.period(Duration::from_secs(60));
        wait(&clock, 80);
        assert_eq!(
            count(&counter),
            1,
            "the twenty-millisecond chain kept ticking after the period changed"
        );

        wait(&clock, 60_000);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_timer_ticks_only_while_active() {
        let clock = clock();
        let (_ticking, timer, counter) = counting(30);

        let active = Arc::new(AtomicBool::new(false));
        let a = active.clone();
        let _timer = timer.when(move || a.load(Ordering::SeqCst));

        wait(&clock, 60);
        assert_eq!(count(&counter), 0);

        active.store(true, Ordering::SeqCst);
        wait(&clock, 30);
        assert_eq!(count(&counter), 1);

        active.store(false, Ordering::SeqCst);
        wait(&clock, 60);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn a_timer_skips_its_ticks_while_its_scope_sleeps() {
        let clock = clock();
        let scope = guinea_core::scope::Scope::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        let (_ticking, _timer) = start(
            Location::caller(),
            None,
            Some((0, scope.awake())),
            Duration::from_millis(30).into(),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );

        wait(&clock, 30);
        assert_eq!(count(&counter), 1);

        scope.sleep();
        wait(&clock, 90);
        assert_eq!(count(&counter), 1, "ticked while its scope slept");

        scope.wake();
        wait(&clock, 30);
        assert_eq!(
            count(&counter),
            2,
            "the ticks it slept through were saved up for it"
        );
    }

    type Watcher = (u64, Arc<dyn Fn() + Send + Sync>);

    /// A value set by hand, telling whoever watches it, from the thread that
    /// set it.
    #[derive(Clone, Default)]
    struct Knob(Arc<std::sync::Mutex<(Option<u64>, Vec<Watcher>, u64)>>);

    impl Knob {
        fn set(&self, ms: u64) {
            let watchers: Vec<Watcher> = {
                let mut knob = self.0.lock().unwrap();
                knob.0 = Some(ms);
                knob.1.clone()
            };
            for (_, changed) in watchers {
                changed();
            }
        }

        fn watchers(&self) -> usize {
            self.0.lock().unwrap().1.len()
        }
    }

    struct Unwatch(Knob, u64);

    impl Drop for Unwatch {
        fn drop(&mut self) {
            let id = self.1;
            self.0.0.lock().unwrap().1.retain(|(watcher, _)| *watcher != id);
        }
    }

    impl Changing for Knob {
        type Value = u64;

        fn now(&self) -> Option<u64> {
            self.0.lock().unwrap().0
        }

        fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn std::any::Any> {
            let mut knob = self.0.lock().unwrap();
            knob.2 += 1;
            let id = knob.2;
            knob.1.push((id, Arc::from(changed)));
            Box::new(Unwatch(self.clone(), id))
        }
    }

    fn following(knob: &Knob) -> (Ticking, Timer, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        let (ticking, timer) = start(
            Location::caller(),
            None,
            None,
            Period::follows(knob.clone(), Duration::from_millis),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );

        (ticking, timer, counter)
    }

    #[test]
    fn a_period_that_follows_a_value_changes_the_moment_the_value_does() {
        let clock = clock();
        let knob = Knob::default();
        knob.set(1_000);
        let (_ticking, _timer, counter) = following(&knob);

        wait(&clock, 100);
        knob.set(50);

        wait(&clock, 50);
        assert_eq!(count(&counter), 1, "the second it had left was waited out");

        wait(&clock, 50);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_period_with_nothing_to_follow_waits_for_something() {
        let clock = clock();
        let knob = Knob::default();
        let (_ticking, _timer, counter) = following(&knob);

        wait(&clock, 10_000);
        assert_eq!(count(&counter), 0, "ticked with no period to tick by");

        knob.set(30);
        wait(&clock, 30);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn what_a_period_follows_is_watched_for_as_long_as_the_timer_runs() {
        let _clock = clock();
        let knob = Knob::default();
        knob.set(1_000);

        let (ticking, _timer, _counter) = following(&knob);
        assert_eq!(knob.watchers(), 1);

        drop(ticking);
        assert_eq!(knob.watchers(), 0, "the timer stopped and kept watching");
    }

    #[test]
    fn a_tick_may_list_the_running_timers_itself() {
        let clock = clock();
        let seen = Arc::new(AtomicUsize::new(0));

        let s = seen.clone();
        let (_ticking, _timer) = start(
            Location::caller(),
            None,
            None,
            Duration::from_millis(30).into(),
            move || {
                s.store(running().len(), Ordering::SeqCst);
            },
        );

        wait(&clock, 30);
        assert_eq!(count(&seen), 1);
    }

    #[test]
    fn a_running_timer_says_where_it_was_set_up_and_what_it_is_called() {
        let _clock = clock();
        let (ticking, timer, _counter) = counting(1_000);
        let (_kept, hidden, _) = counting(1_000);

        let timer = timer.named("sweep");
        let hidden = hidden.untraced().id();

        let id = timer.id().expect("running");
        let listed = running();
        let info = listed.iter().find(|info| info.id == id).expect("listed");
        assert_eq!(info.name, Some("sweep"));
        assert!(info.place.file().ends_with("timers.rs"));
        assert_eq!(info.period, Duration::from_millis(1_000));
        assert!(listed.iter().all(|info| Some(info.id) != hidden));

        drop(ticking);
        assert!(running().iter().all(|info| info.id != id));
        assert_eq!(timer.id(), None);
    }
}
