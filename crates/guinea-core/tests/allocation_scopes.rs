#![cfg(feature = "allocations")]

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll};

use guinea_core::scope::ScopeTree;
use guinea_core::trace::allocations::{Allocator, Config, Operation, Profiler, Stats, allocator};
use guinea_core::trace::{self, Point, Trace};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

fn stats(profiler: &Profiler, id: u64) -> Stats {
    profiler
        .snapshot()
        .into_iter()
        .find(|stats| stats.id == id)
        .unwrap()
}

#[test]
fn scopes_preserve_allocation_origins_across_threads_and_removal() {
    let profiler = Profiler::install(Config {
        history_capacity: 64,
    })
    .unwrap();
    let tree = ScopeTree::new();
    let first = tree.child();
    let second = tree.child();
    let first_id = first.allocation_group().unwrap().id();
    let second_id = second.allocation_group().unwrap().id();
    let root_id = tree.allocation_group().unwrap().id();
    let other_tree = ScopeTree::new();
    assert_eq!(tree.id(), other_tree.id());
    assert_ne!(root_id, other_tree.allocation_group().unwrap().id());
    assert_eq!(stats(&profiler, first_id).parent, Some(root_id));

    let kept = first.track_allocations(|| Box::new([0_u8; 512]));
    assert_eq!(stats(&profiler, first_id).live_bytes, 512);
    assert_eq!(stats(&profiler, first_id).allocations, 1);
    assert!(stats(&profiler, first_id).wrapped_live_bytes > 512);
    first.remove();
    assert!(stats(&profiler, first_id).closed);
    assert_eq!(stats(&profiler, first_id).live_bytes, 512);
    second.track_allocations(|| drop(kept));
    assert_eq!(stats(&profiler, first_id).live_bytes, 0);
    assert_eq!(stats(&profiler, first_id).freed_bytes, 512);
    assert_eq!(stats(&profiler, second_id).freed_bytes, 0);
    let freed = profiler
        .take_history()
        .events
        .into_iter()
        .find(|event| event.operation == Operation::Free)
        .unwrap();
    assert_eq!(freed.group, first_id);
    assert_eq!(freed.current_group, Some(second_id));

    let sink = trace::sink_of(second).unwrap();
    let _recording = trace::record_into(&sink);
    let action = trace::enter(|| Point::Action {
        message: "Allocate",
    });
    let cause = trace::current();
    let work = second.track_allocations(|| {
        let mut yielded = false;
        trace::within(cause, async move {
            let before = Box::new([0_u8; 256]);
            std::future::poll_fn(move |context| {
                if std::mem::replace(&mut yielded, true) {
                    Poll::Ready(())
                } else {
                    context.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await;
            (before, Box::new([0_u8; 1024]))
        })
    });
    drop(action);
    let work = std::thread::spawn(move || {
        let mut work = Box::pin(work);
        assert!(
            work.as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
        work
    })
    .join()
    .unwrap();
    let result = std::thread::spawn(move || {
        let mut work = work;
        match work
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
        {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("the second poll should finish"),
        }
    })
    .join()
    .unwrap();
    assert_eq!(stats(&profiler, second_id).live_bytes, 1280);
    let history = profiler.take_history();
    assert_eq!(history.events.len(), 2);
    assert!(
        history
            .events
            .iter()
            .all(|event| event.group == second_id && event.cause == cause)
    );
    second.remove();
    std::thread::spawn(move || drop(result)).join().unwrap();
    assert_eq!(stats(&profiler, second_id).live_bytes, 0);
    assert_eq!(stats(&profiler, second_id).wrapped_live_bytes, 0);
    assert_eq!(stats(&profiler, second_id).peak_bytes, 1280);

    let current = tree.child();
    let current_id = current.allocation_group().unwrap().id();
    current.track_allocations(|| {
        for _ in 0..100 {
            let block = Box::new([0_u8; 32]);
            std::hint::black_box(&block);
            drop(block);
        }
    });
    let history = profiler.take_history();
    assert_eq!(history.events.len(), 64);
    assert!(history.dropped >= 136);
    assert!(
        history
            .events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert_eq!(stats(&profiler, current_id).allocated_bytes, 3200);
    assert_eq!(stats(&profiler, current_id).live_bytes, 0);

    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorded = seen.clone();
    let _observer = sink.observe(move |trace| {
        if let Trace::Mark(record) = trace {
            let allocation = Box::new([0_u8; 4096]);
            std::hint::black_box(&allocation);
            recorded.borrow_mut().push(record.clone());
        }
    });
    let block = current.track_allocations(|| Box::new([0_u8; 96]));
    drop(block);
    let before = profiler.snapshot();
    assert_eq!(profiler.flush(), 0);
    assert_eq!(profiler.snapshot(), before);
    assert_eq!(seen.borrow().len(), 2);
    assert!(
        seen.borrow()
            .iter()
            .all(|record| matches!(record.point.kind(), "memory.allocate" | "memory.free"))
    );

    let held_by_observer = Rc::new(RefCell::new(Some(
        current.track_allocations(|| Box::new([0_u8; 160])),
    )));
    let held = held_by_observer.clone();
    let _dropping = sink.observe(move |_| {
        drop(held.borrow_mut().take());
    });
    profiler.flush();
    assert!(held_by_observer.borrow().is_none());
    assert_eq!(stats(&profiler, current_id).live_bytes, 0);
    assert!(profiler.take_history().events.iter().any(|event| {
        event.operation == Operation::Free && event.group == current_id && event.bytes == 160
    }));

    let after_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        current.track_allocations(|| panic!("restore allocation context"));
    }));
    assert!(after_panic.is_err());
    let before = stats(&profiler, current_id).allocations;
    let outside = Box::new([0_u8; 24]);
    std::hint::black_box(&outside);
    drop(outside);
    assert_eq!(stats(&profiler, current_id).allocations, before);

    let parallel = tree.child();
    let group = parallel.allocation_group().unwrap();
    std::thread::scope(|threads| {
        for _ in 0..4 {
            let group = group.clone();
            threads.spawn(move || {
                for _ in 0..64 {
                    group.run(|| {
                        let block = Box::new([0_u8; 64]);
                        std::hint::black_box(&block);
                        drop(block);
                    });
                }
            });
        }
    });
    let totals = stats(&profiler, group.id());
    assert_eq!(totals.allocations, 256);
    assert_eq!(totals.allocated_bytes, 16384);
    assert_eq!(totals.live_bytes, 0);
    assert!(Profiler::install(Config::default()).is_err());
}
