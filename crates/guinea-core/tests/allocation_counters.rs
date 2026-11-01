#![cfg(feature = "allocations")]

use guinea_core::scope::ScopeTree;
use guinea_core::trace::allocations::{Allocator, Config, Profiler, allocator};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

#[test]
fn counters_do_not_require_history() {
    let profiler = Profiler::install(Config::default()).unwrap();
    let tree = ScopeTree::new();
    let id = tree.allocation_group().unwrap().id();
    let mut bytes = tree.track_allocations(|| Vec::<u8>::with_capacity(32));
    bytes.extend([1; 32]);
    tree.track_allocations(|| bytes.reserve_exact(96));
    drop(bytes);
    let stats = profiler
        .snapshot()
        .into_iter()
        .find(|stats| stats.id == id)
        .unwrap();
    assert_eq!(stats.live_bytes, 0);
    assert_eq!(stats.wrapped_live_bytes, 0);
    assert!(stats.allocated_bytes >= 160);
    assert_eq!(stats.allocated_bytes, stats.freed_bytes);
    assert!(profiler.take_history().events.is_empty());
    assert_eq!(profiler.flush(), 0);
}
