//! Experimental, application-wide reducer recording. Normal mode never
//! captures values. Detailed mode is explicitly enabled by the caller and
//! captures bounded JSON snapshots before and after each successful reduction.
//! This records reductions, not proof that a value actually changed.
//!
//! The recorder owns a bounded history; scopes keep only a weak reference.
//! Dropping the last recorder stops capture. Seed values and direct calls to
//! `Reducer::reduce` are not recorded. Snapshots honour serde's skip/redaction
//! rules; `Debug` and the inspector's truncated previews are never substituted.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::{Rc, Weak};
use std::time::Duration;

use crate::scope::{Scope, ScopeId};
use crate::trace::{self, Cause, Point};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Detailed,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub capacity: usize,
    /// Maximum encoded JSON bytes per state or update snapshot.
    pub snapshot_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            capacity: 256,
            snapshot_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Snapshot {
    Json(serde_json::Value),
    Unavailable,
    LimitExceeded,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshots {
    pub update: Snapshot,
    pub before: Snapshot,
    pub after: Snapshot,
    /// RFC 6902 JSON Patch; an empty array means the captured state is unchanged.
    pub diff: Snapshot,
}

impl Snapshots {
    pub(crate) fn capture(
        before: Snapshot,
        update: Snapshot,
        after: Snapshot,
        limit: usize,
    ) -> Self {
        let diff = match (&before, &after) {
            (Snapshot::Json(before), Snapshot::Json(after)) => {
                capture_json(&json_patch::diff(before, after), limit)
            }
            _ => Snapshot::Unavailable,
        };
        Self {
            before,
            update,
            after,
            diff,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reduction {
    /// Process-wide recording identity, independent of scope id reuse.
    pub id: Cause,
    pub scope: ScopeId,
    pub reducer: &'static str,
    pub update: &'static str,
    /// Counts applied updates to this state slot, including unrecorded ones.
    /// Seeding a replacement slot resets its revision.
    pub revision: u64,
    pub at: Duration,
    pub cause: Option<Cause>,
    pub snapshots: Option<Snapshots>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    ModeChanged { at: Duration, from: Mode, to: Mode },
    Reduced(Reduction),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub events: Vec<Event>,
    /// Oldest entries overwritten since the previous drain.
    pub dropped: u64,
}

pub(crate) struct Data {
    config: Config,
    mode: Mode,
    events: VecDeque<Event>,
    dropped: u64,
}

impl Data {
    fn append(&mut self, event: Event) {
        if self.events.len() == self.config.capacity {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }
}

/// One recording policy for every scope in an application tree. Starts in
/// normal mode; changing mode affects subsequent reductions only.
#[derive(Clone)]
pub struct Recording(Rc<RefCell<Data>>);

impl Recording {
    /// Panics for a zero budget, a removed scope, or a second active recorder
    /// in the same tree. Independent application trees have independent modes.
    pub fn start(scope: Scope, config: Config) -> Self {
        assert!(scope.is_alive(), "recording states of a removed scope");
        assert!(
            config.capacity > 0 && config.snapshot_bytes > 0,
            "recording budgets must be positive"
        );
        let recording = Self(Rc::new(RefCell::new(Data {
            config,
            mode: Mode::Normal,
            events: VecDeque::new(),
            dropped: 0,
        })));
        scope.record_states(&recording);
        recording
    }

    pub fn mode(&self) -> Mode {
        self.0.borrow().mode
    }

    pub fn set_mode(&self, to: Mode) {
        let mut data = self.0.borrow_mut();
        let from = data.mode;
        if from != to {
            data.mode = to;
            data.append(Event::ModeChanged {
                at: trace::now(),
                from,
                to,
            });
        }
    }

    /// Takes ownership of recorded snapshots rather than cloning them.
    pub fn take(&self) -> History {
        let mut data = self.0.borrow_mut();
        History {
            events: data.events.drain(..).collect(),
            dropped: std::mem::take(&mut data.dropped),
        }
    }

    pub(crate) fn from_data(data: Rc<RefCell<Data>>) -> Self {
        Self(data)
    }

    pub(crate) fn downgrade(&self) -> Weak<RefCell<Data>> {
        Rc::downgrade(&self.0)
    }

    pub(crate) fn capture_limit(&self) -> Option<usize> {
        let data = self.0.borrow();
        (data.mode == Mode::Detailed).then_some(data.config.snapshot_bytes)
    }

    pub(crate) fn reduced(&self, scope: Scope, entry: Reduction) {
        let cause = entry.cause;
        let reducer = entry.reducer;
        let update = entry.update;
        let revision = entry.revision;
        let detailed = entry.snapshots.is_some();
        let id = entry.id;
        let change = match entry.snapshots.as_ref().map(|snapshots| &snapshots.diff) {
            None => "not_captured",
            Some(Snapshot::Json(value)) if value.as_array().is_some_and(Vec::is_empty) => {
                "unchanged"
            }
            Some(Snapshot::Json(_)) => "changed",
            _ => "unavailable",
        };
        let payload = entry.snapshots.as_ref().map(|snapshots| {
            serde_json::json!({
                "schema_version": 1, "update_id": id.get(), "scope": entry.scope.get(),
                "reducer": reducer, "update_type": update, "revision": revision,
                "cause": cause.map(Cause::get), "before": snapshots.before,
                "update": snapshots.update, "after": snapshots.after, "diff": snapshots.diff,
            })
            .to_string()
        });
        self.0.borrow_mut().append(Event::Reduced(entry));
        if let Some(sink) = trace::sink_of(scope) {
            let _recording = trace::record_into(&sink);
            trace::mark_under(cause, || Point::Plugin {
                kind: "state.update",
                fields: vec![
                    ("scope", scope.id().get().to_string()),
                    ("reducer", reducer.to_string()),
                    ("update", update.to_string()),
                    ("revision", revision.to_string()),
                    ("detailed", detailed.to_string()),
                    ("update_id", id.get().to_string()),
                    ("change", change.to_string()),
                ],
            });
            if let Some(payload) = payload {
                trace::mark_under(cause, || Point::Plugin {
                    kind: "state.payload",
                    fields: vec![("update_id", id.get().to_string()), ("json", payload)],
                });
            }
        }
    }
}

struct Bounded {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

fn capture_json(value: &(impl serde::Serialize + ?Sized), limit: usize) -> Snapshot {
    let mut writer = Bounded {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Snapshot::LimitExceeded;
    }
    match result {
        Err(error) => Snapshot::Failed(error.to_string()),
        Ok(()) => match serde_json::from_slice(&writer.bytes) {
            Ok(value) => Snapshot::Json(value),
            Err(error) => Snapshot::Failed(error.to_string()),
        },
    }
}

impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("snapshot byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[doc(hidden)]
pub mod __private {
    use super::{Snapshot, capture_json};

    pub struct Probe<'a, T: ?Sized>(pub &'a T);

    pub trait ViaSerialize {
        fn capture(&self, limit: usize) -> Snapshot;
    }

    impl<T: serde::Serialize + ?Sized> ViaSerialize for &Probe<'_, T> {
        fn capture(&self, limit: usize) -> Snapshot {
            capture_json(self.0, limit)
        }
    }

    pub trait ViaNothing {
        fn capture(&self, limit: usize) -> Snapshot;
    }

    impl<T: ?Sized> ViaNothing for Probe<'_, T> {
        fn capture(&self, _: usize) -> Snapshot {
            Snapshot::Unavailable
        }
    }
}
