#![cfg(feature = "allocations")]

use std::time::Duration;

use guinea::app::Harness;
use guinea::observability::trace::allocations::{
    Allocator, Config, Operation, Profiler, allocator,
};
use guinea::observability::trace::{self, Point, Trace};
use guinea::prelude::*;

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

#[derive(Clone, Debug, Default, PartialEq)]
struct State {
    bytes: Vec<u8>,
}

#[reducer]
fn state(this: &mut State, bytes: Vec<u8>) {
    this.bytes = bytes;
}

struct Start;
struct Queue(u8);
struct Queued;

#[derive(Debug)]
struct Worker {
    push: Push<State>,
}

actor! {
    Worker {
        handlers { Start, Queue }
    }
}

#[handler]
fn start(In(this, _, cx): In<Worker, Start>) {
    cx.task().pushing(this.push).spawn(async {
        let before_wait = vec![1_u8; 8192];
        tokio::time::sleep(Duration::from_millis(10)).await;
        let after_wait = vec![2_u8; 4096];
        std::hint::black_box(&before_wait);
        after_wait
    });
}

#[handler]
fn queue(In(this, Queue(index), cx): In<Worker, Queue>) {
    cx.task()
        .lane::<Queued>()
        .conflate()
        .pushing(this.push)
        .spawn(async move {
            let bytes = vec![index; 16384 + index as usize];
            tokio::time::sleep(Duration::from_millis(10)).await;
            bytes
        });
}

feature! {
    Allocating {
        exports { State }
    }
}

#[installs]
fn allocating(cx: &Install) -> anyhow::Result<Allocating> {
    let (state, _) = cx.state::<State>().driven_by(|push| Worker { push });
    Ok(Allocating(state))
}

#[test]
fn tasks_keep_scope_and_spawn_cause_under_the_seeded_executor() {
    let profiler = Profiler::install(Config {
        history_capacity: 4096,
    })
    .unwrap();
    for seed in 0..8 {
        let harness = Harness::new(seed);
        harness.install::<Allocating>(&()).unwrap();
        let scope = harness.segment().scope();
        let group = scope.allocation_group().unwrap().id();
        let sink = trace::sink_of(scope).unwrap();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let traces = seen.clone();
        let observer = sink.observe(move |record| {
            if let Trace::Mark(record) = record {
                traces.borrow_mut().push(record.clone());
            }
        });
        profiler.take_history();
        harness.dispatch::<State>().emit(Start);
        harness.settled();
        harness.advance(Duration::from_millis(10));
        assert_eq!(harness.state::<State>().bytes.len(), 4096);
        let history = profiler.take_history();
        let spawned = seen
            .borrow()
            .iter()
            .find_map(|record| matches!(record.point, Point::Spawn { .. }).then_some(record.id))
            .unwrap();
        for bytes in [8192, 4096] {
            let event = history
                .events
                .iter()
                .find(|event| event.operation == Operation::Allocate && event.bytes == bytes)
                .expect("the task allocation is recorded");
            assert_eq!(event.group, group);
            assert_eq!(event.cause, Some(spawned));
        }
        harness.dispatch::<State>().emit(Queue(1));
        harness.dispatch::<State>().emit(Queue(2));
        harness.settled();
        harness.advance(Duration::from_millis(10));
        harness.advance(Duration::from_millis(10));
        assert_eq!(harness.state::<State>().bytes.len(), 16386);
        let queued = profiler.take_history();
        let spawned = seen
            .borrow()
            .iter()
            .filter_map(|record| matches!(record.point, Point::Spawn { .. }).then_some(record.id))
            .collect::<Vec<_>>();
        for (bytes, cause) in [16385, 16386]
            .into_iter()
            .zip(&spawned[spawned.len() - 2..])
        {
            let event = queued
                .events
                .iter()
                .find(|event| event.operation == Operation::Allocate && event.bytes == bytes)
                .expect("the queued task allocation is recorded");
            assert_eq!(event.group, group);
            assert_eq!(event.cause, Some(*cause));
        }
        drop(observer);
        drop(seen);
        let child = harness.child();
        child.install::<Allocating>(&()).unwrap();
        let child_group = child.scope().allocation_group().unwrap().id();
        profiler.take_history();
        child.dispatch::<State>().emit(Start);
        harness.settled();
        assert!(profiler.take_history().events.iter().any(|event| {
            event.operation == Operation::Allocate
                && event.group == child_group
                && event.bytes == 8192
        }));
        child.leave();
        harness.settled();
        assert!(profiler.take_history().events.iter().any(|event| {
            event.operation == Operation::Free && event.group == child_group && event.bytes == 8192
        }));
        drop(harness);
        for id in [group, child_group] {
            let stats = profiler
                .snapshot()
                .into_iter()
                .find(|stats| stats.id == id)
                .unwrap();
            assert!(stats.closed);
            assert_eq!(stats.live_bytes, 0);
        }
    }
}
