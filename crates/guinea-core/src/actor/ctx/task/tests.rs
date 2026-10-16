use std::cell::RefCell;
use std::rc::Rc;

use tokio::sync::oneshot;

use super::*;
use crate::actor::In;
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
    fn handle(_: In<Self, Search>) {}
}

impl Handler<Found> for Searcher {
    fn handle(In(this, Found(n), _): In<Self, Found>) {
        this.found.borrow_mut().push(n);
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
        let noted = ends.clone();
        let observing = home.sink().observe(move |trace| {
            if let Trace::Mark(record) = trace
                && ENDS.contains(&record.point.kind())
            {
                noted.borrow_mut().push(record.point.kind());
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

#[derive(Clone, Default, Debug)]
struct Tally(u32);

#[guinea_macros::reducer]
fn tally(this: &mut Tally, by: u32) {
    this.0 += by;
}

#[derive(Clone, Default, Debug)]
struct Names(Vec<Rc<str>>);

#[guinea_macros::reducer]
fn names(this: &mut Names, name: Rc<str>) {
    this.0.push(name);
}

fn tallied(scope: Scope) -> u32 {
    scope.state::<Tally>().borrow().0
}

/// A number held at its first await until `open` is sent.
fn held(n: u32) -> (oneshot::Sender<()>, impl Future<Output = u32> + Send + 'static) {
    let (open, opened) = oneshot::channel::<()>();
    let work = async move {
        let _ = opened.await;
        n
    };

    (open, work)
}

struct Countdown(u32);

impl Stream for Countdown {
    type Item = u32;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<u32>> {
        if self.0 == 0 {
            return std::task::Poll::Ready(None);
        }

        self.0 -= 1;
        std::task::Poll::Ready(Some(self.0 + 1))
    }
}

#[test]
fn what_a_pushing_task_returns_goes_into_the_reducer_and_not_to_the_actor() {
    let bench = Bench::new();
    let scope = bench.home.tree.scope();

    bench.cx().task().pushing(Push::<Tally>::new(scope)).spawn(async { 3 });
    bench.executor.run_until_parked();

    assert_eq!((tallied(scope), bench.found()), (3, vec![]));
    assert_eq!(bench.ends(), ["settled"]);
}

#[test]
fn an_answer_that_cannot_cross_threads_is_made_on_the_ui_thread_from_one_that_can() {
    let bench = Bench::new();
    let scope = bench.home.tree.scope();
    let push = Push::<Names>::new(scope);

    bench
        .cx()
        .task()
        .pushing_with(push, |name: String| Rc::<str>::from(name))
        .spawn(async { String::from("Oslo") });
    bench.executor.run_until_parked();

    let names = scope.state::<Names>().borrow().0.clone();
    assert_eq!(names, [Rc::<str>::from("Oslo")]);
}

#[test]
fn work_that_stops_itself_pushes_what_it_returns() {
    let bench = Bench::new();
    let scope = bench.home.tree.scope();

    bench
        .cx()
        .task()
        .pushing(Push::<Tally>::new(scope))
        .spawn_with(|gone| async move { if gone.is_cancelled() { 0 } else { 4 } });
    bench.executor.run_until_parked();

    assert_eq!(tallied(scope), 4);
}

#[test]
fn under_latest_a_push_that_was_already_on_its_way_is_dropped() {
    let bench = Bench::new();
    let scope = bench.home.tree.scope();
    let push = Push::<Tally>::new(scope);
    let (first, a) = held(1);
    let (second, b) = held(10);

    let _ = first.send(());
    bench.cx().task().latest().pushing(push).spawn(a);
    assert!(bench.executor.step(), "the first piece ran to its end");

    bench.cx().task().latest().pushing(push).spawn(b);
    let _ = second.send(());
    bench.executor.run_until_parked();

    assert_eq!(tallied(scope), 10);
    assert_eq!(bench.ends(), ["superseded", "settled"]);
}

#[test]
fn a_push_into_a_scope_that_went_while_the_work_ran_is_dropped() {
    let bench = Bench::new();
    let elsewhere = crate::scope::ScopeTree::new();
    let (open, work) = held(1);

    bench.cx().task().pushing(Push::<Tally>::new(elsewhere.scope())).spawn(work);
    elsewhere.remove();
    let _ = open.send(());
    bench.executor.run_until_parked();

    assert_eq!(bench.ends(), ["cancelled"]);
}

#[test]
fn a_push_from_a_task_is_caused_by_the_task_settling() {
    let bench = Bench::new();
    let scope = bench.home.tree.scope();

    let marks = Rc::new(RefCell::new(Vec::new()));
    let noted = marks.clone();
    let observing = bench.home.sink().observe(move |trace| {
        if let Trace::Mark(record) = trace {
            noted.borrow_mut().push((record.point.kind(), record.id, record.parent));
        }
    });

    bench.cx().task().pushing(Push::<Tally>::new(scope)).spawn(async { 1 });
    bench.executor.run_until_parked();
    drop(observing);

    let marks = marks.borrow();
    let settled = marks.iter().find(|(kind, ..)| *kind == "settled").map(|(_, id, _)| *id);
    let pushed = marks.iter().find(|(kind, ..)| *kind == "push").map(|(.., parent)| *parent);
    assert!(settled.is_some(), "{marks:?}");
    assert_eq!(pushed, Some(settled), "{marks:?}");
}

#[test]
fn a_pushing_source_pushes_every_item_made_an_update_on_the_ui_thread() {
    let bench = Bench::new();
    let scope = bench.home.tree.scope();

    bench
        .cx()
        .task()
        .pushing_with(Push::<Tally>::new(scope), |n: u32| n * 10)
        .pour(Countdown(3), |n| n);
    bench.executor.run_until_parked();

    assert_eq!(tallied(scope), 60);
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
