//! Opt-in Rust heap attribution. Install the allocator in the executable and
//! start the profiler before creating scope trees. Counters describe where
//! memory was allocated, not who currently owns it. Native heaps are outside
//! this allocator. History is bounded and drained outside allocator callbacks.
//!
//! ```
//! use guinea_trace::allocations::{allocator, Allocator, Config, Group, Profiler};
//! use guinea_trace::Sink;
//!
//! #[global_allocator]
//! static ALLOCATOR: Allocator = allocator(std::alloc::System);
//!
//! let profiler = Profiler::install(Config { history_capacity: 1024 })?;
//! let group = Group::new(7, None, Sink::new()).unwrap();
//! let bytes = group.run(|| vec![0_u8; 4096]);
//! assert_eq!(profiler.snapshot()[0].live_bytes, 4096);
//! group.close();
//! drop(bytes);
//! assert_eq!(profiler.snapshot()[0].live_bytes, 0);
//! profiler.flush();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! guinea-core creates groups for scopes automatically with its `allocations`
//! feature. Actors, reducers, feature installation and mounting establish
//! execution contexts; `crate::within` carries the context on every poll.
//! Backend rendering and external executors need explicit scope integration.
//! `Carried::resume` alone restores trace context, not an allocation group.
//! Allocations outside a tracked execution are not included in these totals.
//! Destruction is charged to the original allocation group, even outside a
//! context; new allocations made by a future's destructor need an explicit
//! execution context as well.
//!
//! For dhat, use `Allocator<dhat::Alloc> = allocator(dhat::Alloc)` and start its
//! profiler separately. Scope counters exclude the backend's own allocations;
//! dhat still sees tracking metadata and allocation-header overhead. Its report
//! is separate and is not automatically grouped by scope. The wrapper stores
//! an allocation group header, with alignment padding, even outside profiling.
//! Reallocation follows tracking-allocator's allocate/copy/free semantics and
//! attributes the replacement block to the currently executing scope.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use tracking_allocator::{
    AllocationGroupId, AllocationGroupToken, AllocationRegistry, AllocationTracker,
};

use crate::{Cause, Point, Sink};

pub use tracking_allocator::SetTrackerError;

/// Tracking around a backend whose own bookkeeping is excluded from scope
/// counters. In particular, dhat's stack collection must not become user data.
pub type Allocator<A = System> = tracking_allocator::Allocator<Backend<A>>;

pub const fn allocator<A: GlobalAlloc>(inner: A) -> Allocator<A> {
    tracking_allocator::Allocator::from_allocator(Backend(inner))
}

#[doc(hidden)]
pub struct Backend<A>(A);

thread_local! {
    static BACKEND_ENDING: Cell<bool> = const { Cell::new(false) };
    static BACKEND_LIFETIME: BackendLifetime = const { BackendLifetime };
}

struct BackendLifetime;

impl Drop for BackendLifetime {
    fn drop(&mut self) {
        let _ending = BACKEND_ENDING.try_with(|ending| ending.set(true));
    }
}

fn backend<R>(work: impl FnOnce() -> R) -> R {
    if !is_enabled() || BACKEND_ENDING.try_with(Cell::get).unwrap_or(true) {
        return work();
    }
    let result = AllocationRegistry::untracked(work);
    let _lifetime = BACKEND_LIFETIME.try_with(|_| ());
    result
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for Backend<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        backend(|| unsafe { self.0.alloc(layout) })
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        backend(|| unsafe { self.0.alloc_zeroed(layout) })
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        backend(|| unsafe { self.0.dealloc(pointer, layout) });
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        backend(|| unsafe { self.0.realloc(pointer, layout, size) })
    }
}

/// Zero history capacity keeps counters only. A nonzero capacity keeps the
/// most recent allocation/free events and counts overwritten events.
#[derive(Clone, Copy, Debug, Default)]
pub struct Config {
    pub history_capacity: usize,
}

/// Direct, non-inclusive totals for one scope. IDs are process-wide; `scope`
/// is the original, tree-local scope ID. Closed scopes remain queryable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub id: u64,
    pub scope: u64,
    pub parent: Option<u64>,
    pub closed: bool,
    pub allocations: u64,
    pub deallocations: u64,
    pub allocated_bytes: u64,
    pub freed_bytes: u64,
    pub live_bytes: u64,
    pub peak_bytes: u64,
    pub wrapped_live_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Allocate,
    Free,
}

/// `group` always names the allocation's origin. `current_group` and `cause`
/// describe the operation that allocated or freed it. Addresses may be reused;
/// a truncated history is not a complete allocation lifetime ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub at: Duration,
    pub operation: Operation,
    pub group: u64,
    pub current_group: Option<u64>,
    pub cause: Option<Cause>,
    pub address: usize,
    pub bytes: usize,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub events: Vec<Event>,
    pub dropped: u64,
}

struct ScopeStats {
    stats: Stats,
    sink: Sink,
}

struct ExecutionStats {
    group: u64,
    live: u64,
    finished: bool,
}

struct Data {
    scopes: HashMap<u64, ScopeStats>,
    executions: HashMap<usize, ExecutionStats>,
    history: VecDeque<Event>,
    capacity: usize,
    dropped: u64,
    sequence: u64,
}

impl Data {
    fn event(
        &mut self,
        operation: Operation,
        group: u64,
        current_group: Option<u64>,
        address: usize,
        bytes: usize,
    ) {
        if self.capacity == 0 {
            return;
        }
        self.sequence += 1;
        if self.history.len() == self.capacity {
            self.history.pop_front();
            self.dropped += 1;
        }
        self.history.push_back(Event {
            sequence: self.sequence,
            at: crate::now(),
            operation,
            group,
            current_group,
            cause: crate::current(),
            address,
            bytes,
        });
    }
}

struct State {
    active: AtomicBool,
    data: Mutex<Data>,
}

impl State {
    fn data(&self) -> MutexGuard<'_, Data> {
        self.data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

static STATE: OnceLock<Arc<State>> = OnceLock::new();

/// A single profiling session per process, matching tracking-allocator's
/// one-time global tracker registration. Dropping it stops tracking and
/// releases recorded metadata. Install `Allocator` separately in the binary.
pub struct Profiler {
    state: Arc<State>,
}

impl Profiler {
    pub fn install(config: Config) -> Result<Self, SetTrackerError> {
        let state = AllocationRegistry::untracked(|| {
            Arc::new(State {
                active: AtomicBool::new(true),
                data: Mutex::new(Data {
                    scopes: HashMap::new(),
                    executions: HashMap::new(),
                    history: VecDeque::with_capacity(config.history_capacity),
                    capacity: config.history_capacity,
                    dropped: 0,
                    sequence: 0,
                }),
            })
        });
        AllocationRegistry::set_global_tracker(Tracker(state.clone()))?;
        let _installed = STATE.set(state.clone());
        AllocationRegistry::enable_tracking();
        Ok(Self { state })
    }

    pub fn snapshot(&self) -> Vec<Stats> {
        AllocationRegistry::untracked(|| {
            let mut scopes = self
                .state
                .data()
                .scopes
                .values()
                .map(|entry| entry.stats.clone())
                .collect::<Vec<_>>();
            scopes.sort_by_key(|stats| stats.id);
            scopes
        })
    }

    pub fn take_history(&self) -> History {
        AllocationRegistry::untracked(|| {
            let mut data = self.state.data();
            History {
                events: data.history.drain(..).collect(),
                dropped: std::mem::take(&mut data.dropped),
            }
        })
    }

    /// Publishes buffered events into the original tree's normal trace, with
    /// their original timestamps and causes. Never called by the allocator.
    /// Returns the number of overwritten events, which were not published.
    pub fn flush(&self) -> u64 {
        unattributed(|| {
            let history = self.take_history();
            for event in history.events {
                let origin = self
                    .state
                    .data()
                    .scopes
                    .get(&event.group)
                    .map(|entry| (entry.sink.clone(), entry.stats.scope, entry.stats.parent));
                let Some((sink, scope, parent)) = origin else {
                    continue;
                };
                let _recording = crate::record_into(&sink);
                crate::mark_at(
                    event.cause,
                    || event.at,
                    || Point::Plugin {
                        kind: match event.operation {
                            Operation::Allocate => "memory.allocate",
                            Operation::Free => "memory.free",
                        },
                        fields: vec![
                            ("sequence", event.sequence.to_string()),
                            ("group", event.group.to_string()),
                            ("scope", scope.to_string()),
                            (
                                "parent_group",
                                parent.map(|id| id.to_string()).unwrap_or_default(),
                            ),
                            (
                                "current_group",
                                event
                                    .current_group
                                    .map(|id| id.to_string())
                                    .unwrap_or_default(),
                            ),
                            ("address", event.address.to_string()),
                            ("bytes", event.bytes.to_string()),
                        ],
                    },
                );
            }
            history.dropped
        })
    }
}

impl Drop for Profiler {
    fn drop(&mut self) {
        self.state.active.store(false, Ordering::Release);
        AllocationRegistry::disable_tracking();
        AllocationRegistry::untracked(|| {
            let mut data = self.state.data();
            data.scopes.clear();
            data.executions.clear();
            data.history.clear();
        });
    }
}

/// A scope's identity, safe to carry to another thread without keeping its
/// scope alive. Execution groups are separate so nested and concurrent work
/// can enter the same scope without sharing a mutable allocation token.
#[derive(Clone, Debug)]
pub struct Group {
    id: u64,
}

thread_local! {
    static CURRENT_GROUP: RefCell<Option<Group>> = const { RefCell::new(None) };
}

pub(crate) fn current_group() -> Option<Group> {
    CURRENT_GROUP
        .try_with(|current| current.borrow().clone())
        .ok()
        .flatten()
}

pub fn is_enabled() -> bool {
    STATE
        .get()
        .is_some_and(|state| state.active.load(Ordering::Acquire))
}

impl Group {
    pub fn new(scope: u64, parent: Option<&Group>, sink: Sink) -> Option<Self> {
        let state = STATE.get().filter(|_| is_enabled())?;
        AllocationRegistry::untracked(|| {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let mut data = state.data();
            if !state.active.load(Ordering::Acquire) {
                return None;
            }
            data.scopes.insert(
                id,
                ScopeStats {
                    stats: Stats {
                        id,
                        scope,
                        parent: parent.map(|group| group.id),
                        ..Stats::default()
                    },
                    sink,
                },
            );
            Some(Self { id })
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn close(&self) {
        if let Some(state) = STATE.get() {
            AllocationRegistry::untracked(|| {
                if let Some(scope) = state.data().scopes.get_mut(&self.id) {
                    scope.stats.closed = true;
                }
            });
        }
    }

    pub fn run<R>(&self, work: impl FnOnce() -> R) -> R {
        let Some(mut execution) = Execution::new(self) else {
            return work();
        };
        let _entered = execution.token.enter();
        let _current = Current(CURRENT_GROUP.with(|current| current.replace(Some(self.clone()))));
        work()
    }
}

struct Current(Option<Group>);

fn unattributed<R>(work: impl FnOnce() -> R) -> R {
    let Some(mut token) = AllocationGroupToken::register() else {
        return work();
    };
    let _entered = token.enter();
    let _current = Current(CURRENT_GROUP.with(|current| current.replace(None)));
    work()
}

impl Drop for Current {
    fn drop(&mut self) {
        let previous = self.0.take();
        let _restored = CURRENT_GROUP.try_with(|current| current.replace(previous));
    }
}

struct Execution {
    token: AllocationGroupToken,
    state: Arc<State>,
}

impl Execution {
    fn new(group: &Group) -> Option<Self> {
        let state = STATE.get().filter(|_| is_enabled())?;
        AllocationRegistry::untracked(|| {
            let token = AllocationGroupToken::register()?;
            let mut data = state.data();
            if !state.active.load(Ordering::Acquire) {
                return None;
            }
            data.executions.insert(
                token.id().as_usize().get(),
                ExecutionStats {
                    group: group.id,
                    live: 0,
                    finished: false,
                },
            );
            Some(Self {
                token,
                state: state.clone(),
            })
        })
    }
}

impl Drop for Execution {
    fn drop(&mut self) {
        AllocationRegistry::untracked(|| {
            let mut data = self.state.data();
            let id = self.token.id().as_usize().get();
            if let Some(execution) = data.executions.get_mut(&id) {
                execution.finished = true;
                if execution.live == 0 {
                    data.executions.remove(&id);
                }
            }
        });
    }
}

struct Tracker(Arc<State>);

impl AllocationTracker for Tracker {
    fn allocated(&self, address: usize, bytes: usize, wrapped: usize, group_id: AllocationGroupId) {
        if !self.0.active.load(Ordering::Acquire) {
            return;
        }
        let mut data = self.0.data();
        let Some(execution) = data.executions.get_mut(&group_id.as_usize().get()) else {
            return;
        };
        execution.live += 1;
        let group = execution.group;
        let Some(entry) = data.scopes.get_mut(&group) else {
            return;
        };
        let stats = &mut entry.stats;
        stats.allocations += 1;
        stats.allocated_bytes += bytes as u64;
        stats.live_bytes += bytes as u64;
        stats.wrapped_live_bytes += wrapped as u64;
        stats.peak_bytes = stats.peak_bytes.max(stats.live_bytes);
        data.event(Operation::Allocate, group, Some(group), address, bytes);
    }

    fn deallocated(
        &self,
        address: usize,
        bytes: usize,
        wrapped: usize,
        source: AllocationGroupId,
        current: AllocationGroupId,
    ) {
        if !self.0.active.load(Ordering::Acquire) {
            return;
        }
        let mut data = self.0.data();
        let source_id = source.as_usize().get();
        let Some(execution) = data.executions.get_mut(&source_id) else {
            return;
        };
        execution.live -= 1;
        let group = execution.group;
        if execution.finished && execution.live == 0 {
            data.executions.remove(&source_id);
        }
        let current_group = data
            .executions
            .get(&current.as_usize().get())
            .map(|execution| execution.group);
        let Some(entry) = data.scopes.get_mut(&group) else {
            return;
        };
        let stats = &mut entry.stats;
        stats.deallocations += 1;
        stats.freed_bytes += bytes as u64;
        stats.live_bytes -= bytes as u64;
        stats.wrapped_live_bytes -= wrapped as u64;
        data.event(Operation::Free, group, current_group, address, bytes);
    }
}
