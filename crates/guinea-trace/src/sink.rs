//! Where records go.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tracing::Level;
use tracing::level_filters::LevelFilter;

use crate::point::Fields;
use crate::{Cause, Point, Record, Trace};

type Observer = Rc<dyn Fn(&Trace)>;

type Ring = Arc<dyn Fn() + Send + Sync>;

/// Where the records of one tree go: to its observers, on the thread they
/// observe from, and from every other thread into a queue they take.
///
/// A record goes into the sink current on the thread that makes it - the one
/// [`record_into`] made current, or else the thread's [`thread_default`] - and
/// with none current, to `tracing` alone.
#[derive(Clone)]
pub struct Sink(Arc<Shared>);

struct Shared {
    /// How many observers it has, on every thread together.
    observers: AtomicUsize,
    elsewhere: Mutex<Queue>,
    ring: Mutex<Option<Ring>>,
}

impl Default for Sink {
    fn default() -> Self {
        Self::new()
    }
}

impl Sink {
    pub fn new() -> Self {
        Self(Arc::new(Shared {
            observers: AtomicUsize::new(0),
            elsewhere: Mutex::default(),
            ring: Mutex::default(),
        }))
    }

    /// Hands `observer` every record made into this sink on this thread,
    /// beside any other observer of it, until the returned guard drops or the
    /// thread ends. A thread already tearing its locals down hears nothing.
    pub fn observe(&self, observer: impl Fn(&Trace) + 'static) -> Observing {
        let added = HEARD.try_with(|heard| {
            let id = heard.next.get();
            heard.next.set(id + 1);

            let mut list = heard.list.borrow_mut();
            let mut grown = Vec::clone(&list);
            grown.push(Heard {
                sink: self.clone(),
                id,
                observer: Rc::new(observer),
            });
            *list = Rc::new(grown);

            id
        });

        let Ok(id) = added else {
            return Observing {
                heard: None,
                thread: PhantomData,
            };
        };
        self.0.observers.fetch_add(1, Ordering::Relaxed);

        Observing {
            heard: Some((self.clone(), id)),
            thread: PhantomData,
        }
    }

    /// Whether anything observes it, on any thread.
    pub fn is_observed(&self) -> bool {
        self.0.observers.load(Ordering::Relaxed) > 0
    }

    /// What other threads recorded into it since the last take.
    ///
    /// Kept while it is observed, from the threads that do not observe it; a
    /// thread that observes it hears its own records and leaves none here.
    pub fn take_elsewhere(&self) -> Elsewhere {
        let mut queue = self.0.elsewhere.lock().unwrap_or_else(PoisonError::into_inner);
        let taken = std::mem::take(&mut *queue);

        Elsewhere {
            records: taken.records.into(),
            dropped: taken.dropped,
        }
    }

    /// Calls `ring`, from the thread that recorded it, whenever a record
    /// lands in what [`take_elsewhere`](Self::take_elsewhere) takes and found
    /// it empty; `None` stops it.
    #[doc(hidden)]
    pub fn ring_on_elsewhere(&self, rung: Option<Ring>) {
        *self.0.ring.lock().unwrap_or_else(PoisonError::into_inner) = rung;
    }

    fn is(&self, other: &Sink) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn keep_elsewhere(&self, trace: Trace) {
        let mut queue = self.0.elsewhere.lock().unwrap_or_else(PoisonError::into_inner);

        if queue.records.len() == ELSEWHERE_LIMIT {
            queue.records.pop_front();
            queue.dropped += 1;
        }
        let was_empty = queue.records.is_empty();
        queue.records.push_back((thread_id(), trace));
        drop(queue);

        if !was_empty {
            return;
        }
        let rung = self.0.ring.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(rung) = rung {
            rung();
        }
    }

    /// One of its observers went: with the last, what was kept for them goes.
    fn stopped(&self) {
        if self.0.observers.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.take_elsewhere();
        }
    }
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sink")
            .field("observers", &self.0.observers.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct Heard {
    sink: Sink,
    id: u64,
    observer: Observer,
}

/// This thread's observers, of every sink.
struct Hearing {
    list: RefCell<Rc<Vec<Heard>>>,
    next: Cell<u64>,
}

impl Drop for Hearing {
    fn drop(&mut self) {
        for heard in self.list.get_mut().iter() {
            heard.sink.stopped();
        }
    }
}

thread_local! {
    static HEARD: Hearing = Hearing {
        list: RefCell::new(Rc::new(Vec::new())),
        next: Cell::new(0),
    };

    static INTO: RefCell<Option<Sink>> = const { RefCell::new(None) };

    static DEFAULT: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// An observer of a sink's records on this thread, heard until it drops.
#[must_use = "the observer stops hearing when this drops"]
pub struct Observing {
    heard: Option<(Sink, u64)>,
    thread: PhantomData<Rc<()>>,
}

impl Drop for Observing {
    fn drop(&mut self) {
        let Some((sink, id)) = self.heard.take() else {
            return;
        };

        let removed = HEARD
            .try_with(|heard| {
                let mut list = heard.list.borrow_mut();
                let left: Vec<Heard> = list.iter().filter(|heard| heard.id != id).cloned().collect();
                let removed = left.len() < list.len();
                *list = Rc::new(left);
                removed
            })
            .unwrap_or(false);

        if removed {
            sink.stopped();
        }
    }
}

/// Makes `sink` the one records go into on this thread until the guard
/// drops: for code run on behalf of a tree, whatever thread runs it.
pub fn record_into(sink: &Sink) -> Recording {
    recording(Some(sink.clone()))
}

pub(crate) fn recording(sink: Option<Sink>) -> Recording {
    Recording {
        previous: INTO.try_with(|into| into.replace(sink)).ok().flatten(),
    }
}

/// What [`record_into`] made current, until it drops.
#[must_use = "the sink stops being current when this drops"]
pub struct Recording {
    previous: Option<Sink>,
}

impl Drop for Recording {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _gone = INTO.try_with(|into| into.replace(previous));
    }
}

/// Makes `sink` where this thread's records go while no other is current,
/// until the guard drops: for the thread an application's tree runs on.
pub fn thread_default(sink: &Sink) -> ThreadDefault {
    ThreadDefault {
        sink: sink.clone(),
        previous: DEFAULT
            .try_with(|default| default.replace(Some(sink.clone())))
            .ok()
            .flatten(),
    }
}

/// A thread's default sink, until it drops.
#[must_use = "the sink stops being the thread's default when this drops"]
pub struct ThreadDefault {
    sink: Sink,
    previous: Option<Sink>,
}

impl Drop for ThreadDefault {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _gone = DEFAULT.try_with(|default| {
            let mut default = default.borrow_mut();
            if default.as_ref().is_some_and(|current| current.is(&self.sink)) {
                *default = previous;
            }
        });
    }
}

/// The sink records go into on this thread now, if any.
pub fn current_sink() -> Option<Sink> {
    INTO.try_with(|into| into.borrow().clone())
        .ok()
        .flatten()
        .or_else(|| DEFAULT.try_with(|default| default.borrow().clone()).ok().flatten())
}

fn current_is(test: impl Fn(&Sink) -> bool) -> bool {
    let into = INTO
        .try_with(|into| into.borrow().as_ref().map(&test))
        .ok()
        .flatten();

    into.unwrap_or_else(|| {
        DEFAULT
            .try_with(|default| default.borrow().as_ref().is_some_and(&test))
            .unwrap_or(false)
    })
}

/// Whether the sink current here is observed, on any thread.
pub fn is_observed() -> bool {
    current_is(Sink::is_observed)
}

/// Whether the sink current here is observed on this thread: what is
/// recorded here is heard at once rather than kept for later.
pub fn is_heard_here() -> bool {
    current_is(|sink| {
        HEARD
            .try_with(|heard| heard.list.borrow().iter().any(|heard| heard.sink.is(sink)))
            .unwrap_or(false)
    })
}

/// Whether a point recorded here goes somewhere: to an observer of the sink
/// current here, or to `tracing` as a `guinea::` event of a kind the filter
/// lets through.
pub fn is_recorded() -> bool {
    is_observed() || is_written()
}

fn is_written() -> bool {
    Level::DEBUG <= LevelFilter::current()
        && (tracing::enabled!(target: "guinea::action", Level::DEBUG)
            || tracing::enabled!(target: "guinea::send", Level::DEBUG)
            || tracing::enabled!(target: "guinea::handle", Level::DEBUG)
            || tracing::enabled!(target: "guinea::spawn", Level::DEBUG)
            || tracing::enabled!(target: "guinea::settled", Level::DEBUG)
            || tracing::enabled!(target: "guinea::cancelled", Level::DEBUG)
            || tracing::enabled!(target: "guinea::superseded", Level::DEBUG)
            || tracing::enabled!(target: "guinea::skipped", Level::DEBUG)
            || tracing::enabled!(target: "guinea::source", Level::DEBUG)
            || tracing::enabled!(target: "guinea::arrived", Level::DEBUG)
            || tracing::enabled!(target: "guinea::pull", Level::DEBUG)
            || tracing::enabled!(target: "guinea::closed", Level::DEBUG)
            || tracing::enabled!(target: "guinea::publish", Level::DEBUG)
            || tracing::enabled!(target: "guinea::deliver", Level::DEBUG)
            || tracing::enabled!(target: "guinea::push", Level::DEBUG)
            || tracing::enabled!(target: "guinea::navigate", Level::DEBUG)
            || tracing::enabled!(target: "guinea::tick", Level::DEBUG)
            || tracing::enabled!(target: "guinea::render", Level::DEBUG)
            || tracing::enabled!(target: "guinea::span", Level::DEBUG)
            || tracing::enabled!(target: "guinea::plugin", Level::DEBUG))
}

/// How many records a sink keeps for [`Sink::take_elsewhere`] between two
/// takes; past it the oldest go.
pub const ELSEWHERE_LIMIT: usize = 16_384;

/// Records made into an observed sink on threads that do not observe it.
#[derive(Debug, Default)]
pub struct Elsewhere {
    /// Oldest first, each with the thread it was made on, as [`thread_id`]
    /// names it.
    pub records: Vec<(u32, Trace)>,
    /// How many were let go since the last take, to keep the newest.
    pub dropped: u64,
}

#[derive(Default)]
struct Queue {
    records: VecDeque<(u32, Trace)>,
    dropped: u64,
}

/// This thread, as [`Elsewhere`] names it: the operating system's id on
/// Windows, what a stack sampler names it by; elsewhere a number this
/// process gives each thread once, from 1.
pub fn thread_id() -> u32 {
    thread_local! {
        static ID: u32 = os_thread_id();
    }
    ID.with(|id| *id)
}

#[cfg(windows)]
fn os_thread_id() -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    unsafe { GetCurrentThreadId() }
}

#[cfg(not(windows))]
fn os_thread_id() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Whether `target` is one [`emit`] writes points under, so a layer that
/// turns `tracing` events into points can leave them alone.
pub fn is_point_target(target: &str) -> bool {
    target.starts_with("guinea::")
}

macro_rules! point {
    ($target:literal, $record:expr $(, $($field:tt)+)?) => {
        tracing::debug!(
            target: $target,
            id = $record.id.get(),
            parent = $record.parent.map(Cause::get)
            $(, $($field)+)?
        )
    };
}

fn write(record: &Record) {
    match &record.point {
        Point::Action { message } => point!("guinea::action", record, action = %message),
        Point::Send { actor, message } => {
            point!("guinea::send", record, actor = %actor, msg = %message)
        }
        Point::Handle { actor, message } => {
            point!("guinea::handle", record, actor = %actor, msg = %message)
        }
        Point::Spawn {
            actor,
            actor_id,
            output,
        } => point!(
            "guinea::spawn",
            record,
            actor = %actor,
            actor_id,
            output = %output
        ),
        Point::Settled {
            actor,
            actor_id,
            output,
            took_us,
        } => point!(
            "guinea::settled",
            record,
            actor = %actor,
            actor_id,
            output = %output,
            took_us
        ),
        Point::Cancelled {
            actor,
            actor_id,
            output,
            took_us,
        } => point!(
            "guinea::cancelled",
            record,
            actor = %actor,
            actor_id,
            output = %output,
            took_us
        ),
        Point::Superseded {
            actor,
            actor_id,
            output,
            took_us,
        } => point!(
            "guinea::superseded",
            record,
            actor = %actor,
            actor_id,
            output = %output,
            took_us
        ),
        Point::Skipped {
            actor,
            actor_id,
            output,
        } => point!(
            "guinea::skipped",
            record,
            actor = %actor,
            actor_id,
            output = %output
        ),
        Point::Source {
            source,
            to,
            output,
            file,
            line,
        } => point!(
            "guinea::source",
            record,
            source,
            to = %to,
            output = %output,
            file = *file,
            line = *line
        ),
        Point::Arrived {
            source,
            name,
            to,
            output,
        } => point!(
            "guinea::arrived",
            record,
            source,
            name = *name,
            to = %to,
            output = %output
        ),
        Point::Pull { source, to, output } => point!(
            "guinea::pull",
            record,
            source,
            to = %to,
            output = %output
        ),
        Point::Closed {
            source,
            to,
            output,
            took_us,
            gone,
        } => point!(
            "guinea::closed",
            record,
            source,
            to = %to,
            output = %output,
            took_us,
            gone
        ),
        Point::Publish { event, subscribers } => {
            point!("guinea::publish", record, event = %event, subscribers)
        }
        Point::Deliver { event } => point!("guinea::deliver", record, event = %event),
        Point::Push { reducer } => point!("guinea::push", record, reducer = %reducer),
        Point::Navigate { root, to } => point!("guinea::navigate", record, root = %root, to = %to),
        Point::Render { segment, took_us } => {
            point!("guinea::render", record, segment = %segment, took_us)
        }
        Point::Span {
            name,
            file,
            line,
            module,
            fields,
            ..
        } => point!(
            "guinea::span",
            record,
            span = %name,
            module = *module,
            file = *file,
            line = *line,
            fields = %fields
        ),
        Point::Plugin { kind, fields } => {
            point!("guinea::plugin", record, kind = *kind, fields = %Fields(fields))
        }
        Point::Log { .. } => {}
    }
}

pub(crate) fn emit(trace: Trace) {
    match &trace {
        Trace::Begin(record) | Trace::Mark(record) => write(record),
        Trace::End { id, took } => tracing::trace!(
            target: "guinea::end",
            id = id.get(),
            took_us = took.as_micros() as u64
        ),
    }

    let Some(sink) = current_sink().filter(Sink::is_observed) else {
        return;
    };
    let heard = HEARD.try_with(|heard| heard.list.borrow().clone()).ok();

    let mut told = false;
    for heard in heard.iter().flat_map(|list| list.iter()) {
        if heard.sink.is(&sink) {
            (heard.observer)(&trace);
            told = true;
        }
    }

    if !told {
        sink.keep_elsewhere(trace);
    }
}
