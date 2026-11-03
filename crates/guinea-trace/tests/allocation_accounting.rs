#![cfg(feature = "allocations")]

use guinea_trace::allocations::{Allocator, Config, Group, Profiler, allocator, heap, of_cause};
use guinea_trace::{Point, Sink, enter, record_into};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

#[test]
fn whole_heap_and_direct_causes_survive_scope_exit_thread_hops_and_history_loss() {
    // Startup and allocations made before profiling are already in the total.
    let before = heap();
    let early = Box::new([0_u8; 333]);
    std::hint::black_box(&early);
    assert_eq!(heap().live_bytes - before.live_bytes, 333);
    let profiler = Profiler::install(Config {
        history_capacity: 2,
    })
    .unwrap();
    drop(early); // No origin metadata exists, but the global total still frees it.
    let before = heap();
    let unscoped = Box::new([0_u8; 777]);
    std::hint::black_box(&unscoped);
    assert!(heap().live_bytes >= before.live_bytes + 777);
    drop(unscoped);

    let sink = Sink::new();
    let _current = record_into(&sink);
    let _observer = sink.observe(|_| {
        let temporary = Box::new([0_u8; 1234]);
        std::hint::black_box(&temporary);
    });
    let group = Group::new(42, None, sink.clone()).unwrap();
    let action = enter(|| Point::Action { message: "Parent" });
    let parent = action.id();
    let owned = group.run(|| {
        let bytes = Box::new([0_u8; 4096]);
        std::hint::black_box(&bytes);
        bytes
    });
    let child = enter(|| Point::Handle {
        actor: "Worker",
        message: "Child",
    });
    let child_id = child.id();
    let other = group.run(|| Box::new([0_u8; 2048]));
    drop(child);
    assert_eq!(of_cause(parent).unwrap().allocated_bytes, 4096);
    assert_eq!(of_cause(child_id).unwrap().allocated_bytes, 2048);
    assert_eq!(profiler.snapshot()[0].allocated_bytes, 6144);
    assert!(profiler.tracing().allocated_bytes >= 3 * 1234);
    assert_eq!(profiler.tracing().live_bytes, 0);
    drop(action);
    group.close();
    std::thread::spawn(move || {
        drop(owned);
        drop(other);
    })
    .join()
    .unwrap();
    assert_eq!(of_cause(parent).unwrap().live_bytes, 0);
    assert_eq!(of_cause(parent).unwrap().freed_bytes, 4096);
    assert_eq!(of_cause(child_id).unwrap().live_bytes, 0);
    let history = profiler.take_history();
    assert_eq!(history.events.len(), 2);
    assert_eq!(history.dropped, 2);
    assert_eq!(history.events[0].allocated_by, Some(parent));
    assert_eq!(history.events[1].allocated_by, Some(child_id));
    assert_eq!(history.events[0].cause, None);
    assert_eq!(profiler.snapshot()[0].live_bytes, 0);

    // A cause can be attributed without any scope at all.
    let unscoped = enter(|| Point::Action {
        message: "No scope",
    });
    let id = unscoped.id();
    let mut bytes = Vec::<u8>::with_capacity(32);
    bytes.extend_from_slice(&[1; 32]);
    bytes.reserve_exact(96);
    std::hint::black_box(&bytes);
    drop(bytes);
    drop(unscoped);
    let stats = of_cause(id).unwrap();
    assert!(stats.allocated_bytes >= 160);
    assert_eq!(stats.live_bytes, 0);
    assert_eq!(stats.allocated_bytes, stats.freed_bytes);
    assert_eq!(profiler.snapshot()[0].allocated_bytes, 6144);

    let retained = std::sync::Arc::new(std::sync::Mutex::new(None));
    let output = retained.clone();
    let observing = sink.observe(move |_| {
        *output.lock().unwrap() = Some(Box::new([0_u8; 512]));
    });
    let point = enter(|| Point::Action {
        message: "Observer retention",
    });
    drop(observing);
    drop(point);
    assert_eq!(profiler.tracing().live_bytes, 512);
    drop(retained.lock().unwrap().take());
    assert_eq!(profiler.tracing().live_bytes, 0);

    // Address reuse under contention must not remove another allocation's
    // origin. Attribution is removed before the backend releases the address.
    let concurrent = Group::new(43, None, sink).unwrap();
    std::thread::scope(|threads| {
        for _ in 0..4 {
            let group = concurrent.clone();
            threads.spawn(move || {
                for _ in 0..1000 {
                    let block = group.run(|| Box::new([0_u8; 64]));
                    std::hint::black_box(&block);
                    drop(block);
                }
            });
        }
    });
    let stats = profiler
        .snapshot()
        .into_iter()
        .find(|stats| stats.id == concurrent.id())
        .unwrap();
    assert_eq!(stats.allocations, 4000);
    assert_eq!(stats.allocated_bytes, 256_000);
    assert_eq!(stats.freed_bytes, stats.allocated_bytes);
    assert_eq!(stats.live_bytes, 0);
}
