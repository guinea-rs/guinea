//! What publishing an event costs, by how many hear it and the size of what
//! it carries: heard by callbacks, and heard by actors.

use std::hint::black_box;
use std::rc::Rc;

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use guinea_core::actor::event_bus::{Event, EventBus};
use guinea_core::actor::event_bus::subscribe::BusSubscription;
use guinea_core::__private::Home;
use guinea_core::actor::{Addr, Handler, In};
use guinea_core::scope::ScopeTree;

const HEARD_BY: [usize; 3] = [1, 4, 16];

#[derive(Clone)]
struct Payload<const N: usize>([u8; N]);

impl<const N: usize> Event for Payload<N> {}

#[derive(Debug)]
struct Sink(u64);

guinea_macros::actor! {
    Sink {
        subscribes { Payload<0>, Payload<256>, Payload<4096> }
    }
}

impl<const N: usize> Handler<Payload<N>> for Sink {
    fn handle(In(this, Payload(bytes), _): In<Self, Payload<N>>) {
        this.0 += black_box(bytes).len() as u64;
    }
}

fn by_callbacks<const N: usize>(group: &mut BenchmarkGroup<WallTime>) {
    for heard_by in HEARD_BY {
        let bus = Rc::new(EventBus::new());
        let _subscriptions: Vec<BusSubscription> = (0..heard_by)
            .map(|_| bus.subscribe_fn(|Payload(bytes): Payload<N>| _ = black_box(bytes)))
            .collect();

        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::new(format!("{N}B"), heard_by), |b| {
            b.iter(|| bus.publish(black_box(Payload([0u8; N]))))
        });
    }
}

fn by_actors<const N: usize>(group: &mut BenchmarkGroup<WallTime>) {
    for heard_by in HEARD_BY {
        let tree = ScopeTree::new();
        let bus = tree.bus();
        let home = Home::new(tree.scope());
        let _sinks: Vec<Addr<Sink>> = (0..heard_by).map(|_| Addr::new(Sink(0), &home)).collect();

        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::new(format!("{N}B"), heard_by), |b| {
            b.iter(|| bus.publish(black_box(Payload([0u8; N]))))
        });

        tree.remove();
    }
}

fn publish(c: &mut Criterion) {
    let mut group = c.benchmark_group("callbacks");
    by_callbacks::<0>(&mut group);
    by_callbacks::<256>(&mut group);
    by_callbacks::<4096>(&mut group);
    group.finish();

    let mut group = c.benchmark_group("actors");
    by_actors::<0>(&mut group);
    by_actors::<256>(&mut group);
    by_actors::<4096>(&mut group);
    group.finish();
}

criterion_group!(benches, publish);
criterion_main!(benches);
