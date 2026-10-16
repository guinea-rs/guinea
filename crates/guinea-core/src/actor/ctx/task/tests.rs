use std::cell::RefCell;
use std::rc::Rc;

use tokio::sync::oneshot;

use super::*;
use crate::actor::addr::{Addr, TestHome};
use crate::executor::seeded::{self as executor, Installed};
use crate::trace::Trace;

struct Search;
struct Found(u32);

#[derive(Debug)]
struct Searcher {
    found: Rc<RefCell<Vec<u32>>>,
}

guinea_macros::actor! {
    Searcher {
        handlers {
            Search => { task Found }
            Found
        }
    }
}

impl Handler<Search> for Searcher {
    fn handle(&mut self, _: Search, _cx: Cx<Self, Search>) {}
}

impl Handler<Found> for Searcher {
    fn handle(&mut self, Found(n): Found, _cx: Cx<Self, Found>) {
        self.found.borrow_mut().push(n);
    }
}

/// A search held at its first await until `open` is sent.
fn search(n: u32) -> (oneshot::Sender<()>, impl Future<Output = Found> + Send + 'static) {
    let (open, opened) = oneshot::channel::<()>();
    let work = async move {
        let _ = opened.await;
        Found(n)
    };

    (open, work)
}

const ENDS: [&str; 4] = ["settled", "cancelled", "superseded", "skipped"];

/// What `crates/guinea/tests/lanes.rs` cannot reach through the harness: one
/// step of the executor, and an actor gone while its `Cx` is still in hand.
struct Bench {
    executor: Installed,
    home: TestHome,
    addr: Addr<Searcher>,
    found: Rc<RefCell<Vec<u32>>>,
    ends: Rc<RefCell<Vec<&'static str>>>,
    _observing: trace::Observing,
}

impl Bench {
    fn new() -> Self {
        let executor = executor::install(0);
        let home = TestHome::new();

        let found = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new(Searcher { found: found.clone() }, &home);

        let ends = Rc::new(RefCell::new(Vec::new()));
        let sink = ends.clone();
        let observing = trace::observe(move |trace| {
            if let Trace::Mark(record) = trace
                && ENDS.contains(&record.point.kind())
            {
                sink.borrow_mut().push(record.point.kind());
            }
        });

        Self {
            executor,
            home,
            addr,
            found,
            ends,
            _observing: observing,
        }
    }

    fn cx(&self) -> Cx<Searcher, Search> {
        Cx::new(self.addr.clone())
    }

    fn found(&self) -> Vec<u32> {
        self.found.borrow().clone()
    }

    fn ends(&self) -> Vec<&'static str> {
        self.ends.borrow().clone()
    }
}

#[test]
fn under_latest_an_answer_that_came_before_its_replacement_was_asked_for_is_still_dropped() {
    let bench = Bench::new();
    let (first, a) = search(1);
    let (second, b) = search(2);

    let _ = first.send(());
    bench.cx().task().latest().spawn(a);
    assert!(bench.executor.step(), "the first piece ran to its end");

    bench.cx().task().latest().spawn(b);
    let _ = second.send(());
    bench.executor.run_until_parked();

    assert_eq!(bench.found(), [2]);
    assert_eq!(bench.ends(), ["superseded", "settled"]);
}

#[test]
fn every_spawn_after_the_actor_went_ends_in_the_trace() {
    let bench = Bench::new();
    let (_first, a) = search(1);
    let (_second, b) = search(2);

    bench.home.tree.remove();
    bench.cx().task().conflate().spawn(a);
    bench.cx().task().conflate().spawn(b);
    bench.executor.run_until_parked();

    assert_eq!(bench.ends(), ["cancelled", "cancelled"], "two spawns, so two ends");
}
