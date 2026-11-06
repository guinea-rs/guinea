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
//! Scope totals include only attributed work; [`heap`] counts all requests to
//! the installed Rust allocator, including outside scopes and before profiling.
//! Native heaps and allocator-internal reserved capacity remain outside it.
//! Cause totals are direct (not inclusive of children). Observer allocations
//! are separated from application work. Recursive profiler/backend bookkeeping
//! is present in `heap`, but excluded from attribution to avoid recursion.
//! Causes with outstanding allocations remain queryable; completed causes use
//! a bounded recent cache even when no trace sink is recording. Scope and heap
//! counters are not truncated when a completed cause expires.
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
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use tracking_allocator::{
    AllocationGroupId, AllocationGroupToken, AllocationRegistry, AllocationTracker,
};

use crate::{Cause, Point, Sink};

/// Maximum number of causes retained after their last allocation is freed.
/// Causes with outstanding allocations are retained separately until freed.
/// This limit is independent of [`Config::history_capacity`].
pub const COMPLETED_CAUSE_LIMIT: usize = 4096;

pub use tracking_allocator::SetTrackerError;

/// Tracking around a backend whose own bookkeeping is excluded from scope
/// counters. In particular, dhat's stack collection must not become user data.
pub struct Allocator<A = System> {
    inner: tracking_allocator::Allocator<Backend<A>>,
}

pub const fn allocator<A: GlobalAlloc>(inner: A) -> Allocator<A> {
    Allocator {
        inner: tracking_allocator::Allocator::from_allocator(Backend(inner)),
    }
}

/// Requested Rust bytes, excluding the allocation headers added by this wrapper.
/// Counts begin at process startup, independent of whether a profiler is active.
/// Concurrent snapshots can span several allocator operations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub allocations: u64,
    pub deallocations: u64,
    pub allocated_bytes: u64,
    pub freed_bytes: u64,
    pub live_bytes: u64,
    pub peak_bytes: u64,
}

static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static FREED: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);

pub fn heap() -> Totals {
    Totals {
        allocations: ALLOCS.load(Ordering::Relaxed),
        deallocations: FREES.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED.load(Ordering::Relaxed),
        freed_bytes: FREED.load(Ordering::Relaxed),
        live_bytes: LIVE.load(Ordering::Relaxed),
        peak_bytes: PEAK.load(Ordering::Relaxed),
    }
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for Allocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.inner.alloc(layout) };
        if !pointer.is_null() {
            let bytes = layout.size() as u64;
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED.fetch_add(bytes, Ordering::Relaxed);
            let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // tracking-allocator calls deallocated *after* releasing the block.
        // Remove our address metadata first so address reuse cannot race it.
        if !internal_active() {
            release(pointer as usize, layout.size());
        }
        // Charge before releasing: another thread may immediately reuse the block.
        FREES.fetch_add(1, Ordering::Relaxed);
        FREED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        unsafe { self.inner.dealloc(pointer, layout) };
    }
    // Default alloc_zeroed/realloc use this implementation's alloc/dealloc,
    // preserving tracking-allocator's allocate/copy/free semantics.
}

thread_local! {
    static OBSERVER: Cell<bool> = const { Cell::new(false) };
    static INTERNAL: Cell<bool> = const { Cell::new(false) };
}

fn internal_active() -> bool {
    INTERNAL.try_with(Cell::get).unwrap_or(true)
}

fn internal<R>(work: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = INTERNAL.try_with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(INTERNAL.try_with(|flag| flag.replace(true)).unwrap_or(true));
    // Suppress our callback without touching tracking-allocator's TLS stack.
    // That stack may already be tearing down when a thread frees its last data.
    work()
}

pub(crate) fn observing<R>(work: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = OBSERVER.try_with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(OBSERVER.try_with(|flag| flag.replace(true)).unwrap_or(true));
    work()
}

fn observer() -> bool {
    OBSERVER.try_with(Cell::get).unwrap_or(true)
}

impl Totals {
    fn allocate(&mut self, bytes: usize) {
        self.allocations += 1;
        self.allocated_bytes += bytes as u64;
        self.live_bytes += bytes as u64;
        self.peak_bytes = self.peak_bytes.max(self.live_bytes);
    }
    fn free(&mut self, bytes: usize) {
        self.deallocations += 1;
        self.freed_bytes += bytes as u64;
        self.live_bytes -= bytes as u64;
    }
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
    let result = internal(work);
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
    /// Actual close time on the trace clock, not the time of the next sample.
    pub closed_at: Option<Duration>,
    pub allocations: u64,
    pub deallocations: u64,
    pub allocated_bytes: u64,
    pub freed_bytes: u64,
    pub live_bytes: u64,
    pub peak_bytes: u64,
    pub wrapped_live_bytes: u64,
    pub declarations: Vec<Declaration>,
}

/// Existing lexical descriptions attached by features and mounted segments.
/// One scope can contain several features, so this is not a single scope name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub kind: &'static str,
    pub name: &'static str,
    pub file: Option<&'static str>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub crate_dir: Option<&'static str>,
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
    /// The cause which allocated this block, also on a Free event.
    pub allocated_by: Option<Cause>,
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
    origins: HashMap<usize, Origin>,
    causes: Causes,
    tracing: Totals,
}

#[derive(Clone, Copy)]
struct Origin {
    group: Option<u64>,
    cause: Option<Cause>,
    tracing: bool,
    execution: usize,
    wrapped: usize,
}

// Historical totals must not grow with the total number of messages processed.
// Live origins still need their cause until the last block is freed, including
// on other threads. Only then may that cause enter the bounded recent cache.
struct Causes {
    live: HashMap<Cause, Totals>,
    completed: lru::LruCache<Cause, Totals>,
}

impl Causes {
    fn new() -> Self {
        Self {
            live: HashMap::new(),
            completed: lru::LruCache::new(NonZeroUsize::new(COMPLETED_CAUSE_LIMIT).unwrap()),
        }
    }

    fn allocate(&mut self, cause: Cause, bytes: usize) {
        self.live
            .entry(cause)
            .or_insert_with(|| self.completed.pop(&cause).unwrap_or_default())
            .allocate(bytes);
    }

    fn free(&mut self, cause: Cause, bytes: usize) {
        if let Some(stats) = self.live.get_mut(&cause) {
            stats.free(bytes);
            // Count blocks, not bytes: zero-sized allocations may still exist.
            if stats.allocations == stats.deallocations {
                let stats = self.live.remove(&cause).unwrap();
                self.completed.put(cause, stats);
            }
        }
    }

    fn get(&self, cause: &Cause) -> Option<&Totals> {
        self.live.get(cause).or_else(|| self.completed.peek(cause))
    }

    fn iter(&self) -> impl Iterator<Item = (&Cause, &Totals)> {
        self.live.iter().chain(self.completed.iter())
    }

    fn clear(&mut self) {
        self.live.clear();
        self.completed.clear();
    }
}

impl Data {
    fn event(
        &mut self,
        operation: Operation,
        group: u64,
        current_group: Option<u64>,
        address: usize,
        bytes: usize,
        allocated_by: Option<Cause>,
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
            allocated_by,
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
        let state = internal(|| {
            Arc::new(State {
                active: AtomicBool::new(true),
                data: Mutex::new(Data {
                    scopes: HashMap::new(),
                    executions: HashMap::new(),
                    history: VecDeque::with_capacity(config.history_capacity),
                    capacity: config.history_capacity,
                    dropped: 0,
                    sequence: 0,
                    origins: HashMap::new(),
                    causes: Causes::new(),
                    tracing: Totals::default(),
                }),
            })
        });
        AllocationRegistry::set_global_tracker(Tracker(state.clone()))?;
        let _installed = STATE.set(state.clone());
        AllocationRegistry::enable_tracking();
        Ok(Self { state })
    }

    pub fn snapshot(&self) -> Vec<Stats> {
        internal(|| {
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

    /// Direct totals for causes with outstanding allocations and at most
    /// [`COMPLETED_CAUSE_LIMIT`] recently completed causes, independent of the
    /// allocation history. Freeing elsewhere updates the original cause.
    /// Evicted causes are absent; reusing one starts new totals.
    pub fn causes(&self) -> Vec<(Cause, Totals)> {
        internal(|| {
            let mut values: Vec<_> = self
                .state
                .data()
                .causes
                .iter()
                .map(|(id, stats)| (*id, stats.clone()))
                .collect();
            values.sort_by_key(|(id, _)| *id);
            values
        })
    }

    /// Observer allocations intercepted by tracking-allocator. Recursive internal
    /// bookkeeping is excluded here, but included in the global heap counters.
    pub fn tracing(&self) -> Totals {
        internal(|| self.state.data().tracing.clone())
    }

    pub fn take_history(&self) -> History {
        internal(|| {
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
                            (
                                "allocated_by",
                                event
                                    .allocated_by
                                    .map(|id| id.get().to_string())
                                    .unwrap_or_default(),
                            ),
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
        internal(|| {
            let mut data = self.state.data();
            data.scopes.clear();
            data.executions.clear();
            data.history.clear();
            data.origins.clear();
            data.causes.clear();
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

/// A direct cause summary, without allocating a snapshot of all causes.
/// Returns `None` if the cause has no attributed allocations or its completed
/// statistics have expired from the bounded cache; see [`COMPLETED_CAUSE_LIMIT`].
pub fn of_cause(cause: Cause) -> Option<Totals> {
    let state = STATE.get().filter(|_| is_enabled())?;
    internal(|| state.data().causes.get(&cause).cloned())
}

impl Group {
    pub fn declare(&self, declaration: Declaration) {
        if let Some(state) = STATE.get().filter(|_| is_enabled()) {
            internal(|| {
                if let Some(scope) = state.data().scopes.get_mut(&self.id) {
                    let entries = &mut scope.stats.declarations;
                    if let Some(entry) = entries.iter_mut().find(|entry| {
                        entry.kind == declaration.kind && entry.name == declaration.name
                    }) {
                        *entry = declaration;
                    } else {
                        entries.push(declaration);
                    }
                }
            });
        }
    }

    pub fn new(scope: u64, parent: Option<&Group>, sink: Sink) -> Option<Self> {
        let state = STATE.get().filter(|_| is_enabled())?;
        internal(|| {
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
            internal(|| {
                if let Some(scope) = state.data().scopes.get_mut(&self.id) {
                    scope.stats.closed = true;
                    scope.stats.closed_at.get_or_insert_with(crate::now);
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
    let Some(mut token) = register_token() else {
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

fn register_token() -> Option<AllocationGroupToken> {
    // tracking-allocator 0.4 compares registration ids against a high-water
    // mark. Concurrent, out-of-order registrations can spuriously return None
    // before ids are exhausted. Serialize our registrations to preserve scope
    // attribution under contention.
    static REGISTRATION: Mutex<()> = Mutex::new(());
    let _guard = REGISTRATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    AllocationGroupToken::register()
}

impl Execution {
    fn new(group: &Group) -> Option<Self> {
        let state = STATE.get().filter(|_| is_enabled())?;
        internal(|| {
            let token = register_token()?;
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
        internal(|| {
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
        if internal_active() || !self.0.active.load(Ordering::Acquire) {
            return;
        }
        internal(|| {
            let mut data = self.0.data();
            let tracing = observer();
            let cause = (!tracing).then(crate::current).flatten();
            let execution_id = group_id.as_usize().get();
            let group = if tracing {
                None
            } else {
                data.executions.get_mut(&execution_id).map(|execution| {
                    execution.live += 1;
                    execution.group
                })
            };
            // Unattributed work needs only the atomic whole-heap counters;
            // keep address metadata only when there is an origin to preserve.
            if group.is_none() && cause.is_none() && !tracing {
                return;
            }
            data.origins.insert(
                address,
                Origin {
                    group,
                    cause,
                    tracing,
                    execution: execution_id,
                    wrapped,
                },
            );
            if tracing {
                data.tracing.allocate(bytes);
            }
            if let Some(cause) = cause {
                data.causes.allocate(cause, bytes);
            }
            if let Some(group) = group {
                if let Some(entry) = data.scopes.get_mut(&group) {
                    let stats = &mut entry.stats;
                    stats.allocations += 1;
                    stats.allocated_bytes += bytes as u64;
                    stats.live_bytes += bytes as u64;
                    stats.wrapped_live_bytes += wrapped as u64;
                    stats.peak_bytes = stats.peak_bytes.max(stats.live_bytes);
                }
                data.event(
                    Operation::Allocate,
                    group,
                    Some(group),
                    address,
                    bytes,
                    cause,
                );
            }
        });
    }

    fn deallocated(
        &self,
        _: usize,
        _: usize,
        _: usize,
        _: AllocationGroupId,
        _: AllocationGroupId,
    ) {
        // Accounted by the outer allocator before the address can be reused.
    }
}

fn release(address: usize, bytes: usize) {
    let Some(state) = STATE
        .get()
        .filter(|state| state.active.load(Ordering::Acquire))
    else {
        return;
    };
    internal(|| {
        let mut data = state.data();
        let Some(origin) = data.origins.remove(&address) else {
            return;
        };
        if origin.tracing {
            data.tracing.free(bytes);
        }
        if let Some(cause) = origin.cause {
            data.causes.free(cause, bytes);
        }
        let Some(group) = origin.group else {
            return;
        };
        if let Some(execution) = data.executions.get_mut(&origin.execution) {
            execution.live -= 1;
            if execution.finished && execution.live == 0 {
                data.executions.remove(&origin.execution);
            }
        }
        let current_group = current_group().map(|group| group.id);
        if let Some(entry) = data.scopes.get_mut(&group) {
            let stats = &mut entry.stats;
            stats.deallocations += 1;
            stats.freed_bytes += bytes as u64;
            stats.live_bytes -= bytes as u64;
            stats.wrapped_live_bytes -= origin.wrapped as u64;
        }
        data.event(
            Operation::Free,
            group,
            current_group,
            address,
            bytes,
            origin.cause,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_retention_never_evicts_outstanding_or_zero_sized_allocations() {
        let mut causes = Causes::new();
        let owners: Vec<_> = (0..COMPLETED_CAUSE_LIMIT + 1)
            .map(|_| {
                let cause = Cause::next();
                causes.allocate(cause, 0);
                causes.allocate(cause, 64);
                causes.free(cause, 64);
                cause
            })
            .collect();
        for _ in 0..COMPLETED_CAUSE_LIMIT * 2 {
            let cause = Cause::next();
            causes.allocate(cause, 1);
            causes.free(cause, 1);
        }
        assert_eq!(causes.live.len(), owners.len());
        assert_eq!(causes.completed.len(), COMPLETED_CAUSE_LIMIT);
        for owner in owners {
            let stats = causes.get(&owner).unwrap();
            assert_eq!(stats.live_bytes, 0);
            assert_eq!(stats.allocations, 2);
            assert_eq!(stats.deallocations, 1);
            causes.free(owner, 0);
        }
        assert!(causes.live.is_empty());
        assert_eq!(causes.completed.len(), COMPLETED_CAUSE_LIMIT);
    }

    #[test]
    fn recently_completed_causes_resume_totals_but_evicted_ones_start_fresh() {
        let mut causes = Causes::new();
        let cause = Cause::next();
        causes.allocate(cause, 64);
        causes.free(cause, 64);
        causes.allocate(cause, 32);
        assert_eq!(causes.get(&cause).unwrap().allocated_bytes, 96);
        assert_eq!(causes.get(&cause).unwrap().live_bytes, 32);
        assert!(causes.completed.peek(&cause).is_none());
        causes.free(cause, 32);
        for _ in 0..COMPLETED_CAUSE_LIMIT {
            let next = Cause::next();
            causes.allocate(next, 1);
            causes.free(next, 1);
        }
        assert!(causes.get(&cause).is_none());
        causes.allocate(cause, 8);
        assert_eq!(causes.get(&cause).unwrap().allocated_bytes, 8);
        assert_eq!(causes.get(&cause).unwrap().freed_bytes, 0);
    }
}
