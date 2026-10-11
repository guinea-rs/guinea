//! What a message costs to send and handle, by the size of what it carries:
//! one at a time from outside an actor, queued by a handler and drained in
//! one go, and one at a time while the trace is observed, as with devtools
//! open.

use std::hint::black_box;

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use guinea_core::actor::{Addr, Cx, Handler, Home};
use guinea_core::scope::ScopeTree;
use guinea_core::trace;

const BURST: usize = 1000;

#[derive(Debug)]
struct Sink(u64);

struct Payload<const N: usize>([u8; N]);

guinea_macros::actor! {
    Sink {
        handlers {
            Payload<0>, Payload<16>, Payload<64>, Payload<256>, Payload<1024>, Payload<4096>,
            Burst<0>, Burst<16>, Burst<64>, Burst<256>, Burst<1024>, Burst<4096>
        }
    }
}

impl<const N: usize> Handler<Payload<N>> for Sink {
    fn handle(&mut self, Payload(bytes): Payload<N>, _cx: Cx<Self, Payload<N>>) {
        self.0 += black_box(bytes).len() as u64;
    }
}

struct Burst<const N: usize>;

impl<const N: usize> Handler<Burst<N>> for Sink {
    fn handle(&mut self, _: Burst<N>, cx: Cx<Self, Burst<N>>) {
        let addr = cx.addr();
        for _ in 0..BURST {
            addr.send(Payload([0u8; N]));
        }
    }
}

fn one<const N: usize>(group: &mut BenchmarkGroup<WallTime>, addr: &Addr<Sink>) {
    group.throughput(Throughput::Elements(1));
    group.bench_function(BenchmarkId::from_parameter(N), |b| {
        b.iter(|| addr.send(black_box(Payload([0u8; N]))))
    });
}

fn burst<const N: usize>(group: &mut BenchmarkGroup<WallTime>, addr: &Addr<Sink>) {
    group.throughput(Throughput::Elements(BURST as u64));
    group.bench_function(BenchmarkId::from_parameter(N), |b| b.iter(|| addr.send(Burst::<N>)));
}

macro_rules! every_size {
    ($bench:ident, $group:expr, $addr:expr) => {
        $bench::<0>($group, $addr);
        $bench::<16>($group, $addr);
        $bench::<64>($group, $addr);
        $bench::<256>($group, $addr);
        $bench::<1024>($group, $addr);
        $bench::<4096>($group, $addr);
    };
}

fn send(c: &mut Criterion) {
    let tree = ScopeTree::new();
    let addr = Addr::new(Sink(0), &Home::new(tree.scope()));

    let mut group = c.benchmark_group("one_by_one");
    every_size!(one, &mut group, &addr);
    group.finish();

    let mut group = c.benchmark_group("queued");
    every_size!(burst, &mut group, &addr);
    group.finish();

    trace::observe(|_| {});
    let mut group = c.benchmark_group("observed");
    every_size!(one, &mut group, &addr);
    group.finish();
    trace::stop_observing();
}

criterion_group!(benches, send);
criterion_main!(benches);
