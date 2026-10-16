//! Where records go.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use tracing::Level;
use tracing::level_filters::LevelFilter;

use crate::point::Fields;
use crate::{Cause, Point, Record, Trace};

type Observer = Rc<dyn Fn(&Trace)>;

type Observers = Rc<Vec<(u64, Observer)>>;

struct Slot {
    observers: RefCell<Option<Observers>>,
    next: Cell<u64>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.observers.get_mut().take().is_some() {
            stopped();
        }
    }
}

thread_local! {
    static OBSERVERS: Slot = const {
        Slot {
            observers: RefCell::new(None),
            next: Cell::new(0),
        }
    };
}

static OBSERVED_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Hands every record produced on this thread to `observer`, beside any
/// other observer of it, until the returned guard drops or the thread ends.
/// A thread already tearing its locals down is not observed.
pub fn observe(observer: impl Fn(&Trace) + 'static) -> Observing {
    let added = OBSERVERS.try_with(|slot| {
        let id = slot.next.get();
        slot.next.set(id + 1);

        let mut observers = slot.observers.borrow_mut();
        let first = observers.is_none();
        let mut list = observers.as_deref().cloned().unwrap_or_default();
        list.push((id, Rc::new(observer) as Observer));
        *observers = Some(Rc::new(list));

        (id, first)
    });

    let Ok((id, first)) = added else {
        return Observing {
            id: None,
            thread: PhantomData,
        };
    };
    if first {
        OBSERVED_THREADS.fetch_add(1, Ordering::Relaxed);
    }
    Observing {
        id: Some(id),
        thread: PhantomData,
    }
}

/// An observer of this thread's records, heard until it drops.
#[must_use = "the observer stops hearing when this drops"]
pub struct Observing {
    id: Option<u64>,
    thread: PhantomData<Rc<()>>,
}

impl Drop for Observing {
    fn drop(&mut self) {
        let Some(id) = self.id else {
            return;
        };

        let emptied = OBSERVERS
            .try_with(|slot| {
                let mut observers = slot.observers.borrow_mut();
                let Some(list) = observers.as_ref() else {
                    return false;
                };
                let left: Vec<(u64, Observer)> =
                    list.iter().filter(|(other, _)| *other != id).cloned().collect();
                let emptied = left.is_empty();
                *observers = (!emptied).then(|| Rc::new(left));
                emptied
            })
            .unwrap_or(false);

        if emptied {
            stopped();
        }
    }
}

fn stopped() {
    if OBSERVED_THREADS.fetch_sub(1, Ordering::Relaxed) == 1 {
        take_elsewhere();
    }
}

/// Whether anything observes this thread's records.
pub fn is_observed() -> bool {
    OBSERVERS
        .try_with(|slot| slot.observers.borrow().is_some())
        .unwrap_or(false)
}

/// Whether anything observes the records of any thread.
pub fn is_observed_anywhere() -> bool {
    OBSERVED_THREADS.load(Ordering::Relaxed) > 0
}

/// Whether a point recorded anywhere goes somewhere: to devtools, or to
/// `tracing` as a `guinea::` event of a kind the filter lets through.
pub fn is_recorded_anywhere() -> bool {
    is_observed_anywhere() || is_written()
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

/// How many records [`take_elsewhere`] keeps between two takes; past it the
/// oldest go.
pub const ELSEWHERE_LIMIT: usize = 16_384;

/// Records made on threads nobody observes, while some thread observes.
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

fn elsewhere() -> &'static Mutex<Queue> {
    static ELSEWHERE: OnceLock<Mutex<Queue>> = OnceLock::new();
    ELSEWHERE.get_or_init(Mutex::default)
}

type Ring = Arc<dyn Fn() + Send + Sync>;

fn ring() -> &'static Mutex<Option<Ring>> {
    static RING: OnceLock<Mutex<Option<Ring>>> = OnceLock::new();
    RING.get_or_init(Mutex::default)
}

/// Calls `ring`, from the thread that recorded it, whenever a record lands
/// in what [`take_elsewhere`] takes and found it empty; `None` stops it.
#[doc(hidden)]
pub fn ring_on_elsewhere(rung: Option<Ring>) {
    *ring().lock().unwrap_or_else(PoisonError::into_inner) = rung;
}

fn keep_elsewhere(trace: Trace) {
    let mut queue = elsewhere().lock().unwrap_or_else(PoisonError::into_inner);

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
    let rung = ring().lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(rung) = rung {
        rung();
    }
}

/// What other threads recorded since the last take.
///
/// Kept while any thread observes, from the threads that do not; a thread
/// that observes hears its own records and leaves none here.
pub fn take_elsewhere() -> Elsewhere {
    let mut queue = elsewhere().lock().unwrap_or_else(PoisonError::into_inner);
    let taken = std::mem::take(&mut *queue);

    Elsewhere {
        records: taken.records.into(),
        dropped: taken.dropped,
    }
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

fn observers() -> Option<Observers> {
    OBSERVERS
        .try_with(|slot| slot.observers.borrow().clone())
        .ok()
        .flatten()
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

    match observers() {
        Some(observers) => {
            for (_, observer) in observers.iter() {
                observer(&trace);
            }
        }
        None if is_observed_anywhere() => keep_elsewhere(trace),
        None => {}
    }
}
