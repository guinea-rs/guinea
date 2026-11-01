#![cfg(feature = "allocations")]

use guinea_trace::Sink;
use guinea_trace::allocations::{Allocator, Config, Group, Profiler, allocator};

#[global_allocator]
static ALLOCATOR: Allocator<dhat::Alloc> = allocator(dhat::Alloc);

#[test]
fn scope_counters_can_wrap_dhat_without_reentrant_callbacks() {
    let _dhat = dhat::Profiler::builder().testing().build();
    let profiler = Profiler::install(Config {
        history_capacity: 16,
    })
    .unwrap();
    let group = Group::new(42, None, Sink::new()).unwrap();
    let before = dhat::HeapStats::get();
    let block = group.run(|| Box::new([0_u8; 2048]));
    let during = dhat::HeapStats::get();
    assert!(during.total_bytes >= before.total_bytes + 2048);
    assert_eq!(profiler.snapshot()[0].live_bytes, 2048);
    drop(block);
    let stats = profiler.snapshot().remove(0);
    assert_eq!(stats.live_bytes, 0);
    assert_eq!(stats.allocated_bytes, 2048);
    assert_eq!(stats.freed_bytes, 2048);
    assert_eq!(profiler.take_history().events.len(), 2);
}
