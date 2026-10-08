//! Where records go.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use tracing::Level;

use crate::{Cause, Point, Record, Trace};

pub type Observer = Rc<dyn Fn(&Trace)>;

thread_local! {
    static OBSERVER: RefCell<Option<Observer>> = const { RefCell::new(None) };
}

static OBSERVED_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Hands every record produced on this thread to `observer`, replacing any
/// observer already set.
pub fn observe(observer: impl Fn(&Trace) + 'static) {
    let before = OBSERVER.with(|slot| slot.borrow_mut().replace(Rc::new(observer)));
    if before.is_none() {
        OBSERVED_THREADS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn stop_observing() {
    if OBSERVER.with(|slot| slot.borrow_mut().take()).is_some()
        && OBSERVED_THREADS.fetch_sub(1, Ordering::Relaxed) == 1
    {
        take_elsewhere();
    }
}

/// Whether devtools are watching this thread.
pub fn is_observed() -> bool {
    OBSERVER.with(|slot| slot.borrow().is_some())
}

/// Whether devtools are watching any thread.
pub fn is_observed_anywhere() -> bool {
    OBSERVED_THREADS.load(Ordering::Relaxed) > 0
}

/// Whether a point recorded anywhere goes somewhere: to devtools, or to
/// `tracing` as `guinea::` events.
pub fn is_recorded_anywhere() -> bool {
    is_observed_anywhere() || tracing::enabled!(target: "guinea", Level::DEBUG)
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

fn keep_elsewhere(trace: Trace) {
    let mut queue = elsewhere().lock().unwrap_or_else(PoisonError::into_inner);

    if queue.records.len() == ELSEWHERE_LIMIT {
        queue.records.pop_front();
        queue.dropped += 1;
    }
    queue.records.push_back((thread_id(), trace));
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

fn observer() -> Option<Observer> {
    OBSERVER.with(|slot| slot.borrow().clone())
}

pub(crate) fn wanted() -> bool {
    is_recorded_anywhere()
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
        Point::Source {
            actor,
            actor_id,
            output,
        } => point!(
            "guinea::source",
            record,
            actor = %actor,
            actor_id,
            output = %output
        ),
        Point::Arrived {
            actor,
            actor_id,
            output,
            source,
        } => point!(
            "guinea::arrived",
            record,
            actor = %actor,
            actor_id,
            output = %output,
            source
        ),
        Point::Pull {
            actor,
            actor_id,
            output,
            source,
        } => point!(
            "guinea::pull",
            record,
            actor = %actor,
            actor_id,
            output = %output,
            source
        ),
        Point::Closed {
            actor,
            actor_id,
            output,
            took_us,
            gone,
        } => point!(
            "guinea::closed",
            record,
            actor = %actor,
            actor_id,
            output = %output,
            took_us,
            gone
        ),
        Point::Publish {
            event,
            bus,
            subscribers,
        } => point!(
            "guinea::publish",
            record,
            event = %event,
            bus = %bus,
            subscribers
        ),
        Point::Deliver { event, bus } => {
            point!("guinea::deliver", record, event = %event, bus = %bus)
        }
        Point::Push { reducer } => point!("guinea::push", record, reducer = %reducer),
        Point::Navigate { root, to } => point!("guinea::navigate", record, root = %root, to = %to),
        Point::Tick { timer } => point!("guinea::tick", record, timer),
        Point::Store {
            op,
            path,
            field,
            outside,
        } => point!(
            "guinea::store",
            record,
            op = %op,
            path = %path,
            field = field.as_deref(),
            outside
        ),
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
        Point::Note(text) => point!("guinea::note", record, "{text}"),
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

    match observer() {
        Some(observer) => observer(&trace),
        None if is_observed_anywhere() => keep_elsewhere(trace),
        None => {}
    }
}
