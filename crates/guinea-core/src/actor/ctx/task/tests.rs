use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::oneshot;

use super::*;
use crate::actor::addr::{Addr, TestHome};
use crate::executor::{self, Installed};
use crate::trace::Trace;

struct Search;
struct Found(u32);
struct Saving;

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

/// Holds a piece of work at its first await until opened, and tells whether
/// it was ever polled.
struct Gate {
    open: Option<oneshot::Sender<()>>,
    started: Arc<AtomicBool>,
}

impl Gate {
    fn open(&mut self) {
        if let Some(open) = self.open.take() {
            let _ = open.send(());
        }
    }

    fn started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }
}

fn gated<T: Send + 'static>(answer: T) -> (Gate, impl Future<Output = T> + Send + 'static) {
    let (open, opened) = oneshot::channel::<()>();
    let started = Arc::new(AtomicBool::new(false));

    let gate = Gate {
        open: Some(open),
        started: started.clone(),
    };
    let work = async move {
        started.store(true, Ordering::SeqCst);
        let _ = opened.await;
        answer
    };

    (gate, work)
}

fn search(n: u32) -> (Gate, impl Future<Output = Found> + Send + 'static) {
    gated(Found(n))
}

const ENDS: [&str; 4] = ["settled", "cancelled", "superseded", "skipped"];

struct Bench {
    executor: Installed,
    home: TestHome,
    addr: Addr<Searcher>,
    found: Rc<RefCell<Vec<u32>>>,
    ends: Rc<RefCell<Vec<&'static str>>>,
}

impl Bench {
    fn new(seed: u64) -> Self {
        let executor = executor::install(seed);
        let home = TestHome::new();

        let found = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new(Searcher { found: found.clone() }, &home);

        let ends = Rc::new(RefCell::new(Vec::new()));
        let sink = ends.clone();
        trace::observe(move |trace| {
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
        }
    }

    fn cx(&self) -> Cx<Searcher, Search> {
        Cx::new(self.addr.clone())
    }

    fn run(&self) {
        self.executor.run_until_parked();
    }

    fn found(&self) -> Vec<u32> {
        self.found.borrow().clone()
    }

    fn ends(&self) -> Vec<&'static str> {
        self.ends.borrow().clone()
    }

    fn sorted_ends(&self) -> Vec<&'static str> {
        let mut ends = self.ends();
        ends.sort_unstable();
        ends
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        trace::stop_observing();
    }
}

const SEEDS: std::ops::Range<u64> = 0..8;

fn panic_text(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .unwrap_or_default(),
    }
}

#[test]
fn without_a_policy_every_piece_answers() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);

        bench.cx().task().spawn(a);
        bench.cx().task().spawn(b);
        first.open();
        second.open();
        bench.run();

        let mut found = bench.found();
        found.sort_unstable();
        assert_eq!(found, [1, 2], "seed {seed}");
        assert_eq!(bench.ends(), ["settled", "settled"], "seed {seed}");
    }
}

#[test]
fn under_latest_an_answer_that_came_before_its_replacement_was_asked_for_is_still_dropped() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);

        first.open();
        bench.cx().task().latest().spawn(a);
        assert!(bench.executor.step(), "the first piece ran to its end");

        bench.cx().task().latest().spawn(b);
        second.open();
        bench.run();

        assert_eq!(bench.found(), [2], "seed {seed}");
        assert_eq!(bench.ends(), ["superseded", "settled"], "seed {seed}");
    }
}

#[test]
fn under_latest_the_piece_in_flight_is_cut_off() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);

        bench.cx().task().latest().spawn(a);
        bench.run();
        bench.cx().task().latest().spawn(b);
        bench.run();

        first.open();
        second.open();
        bench.run();

        assert_eq!(bench.found(), [2], "seed {seed}");
        assert_eq!(bench.sorted_ends(), ["settled", "superseded"], "seed {seed}");
    }
}

#[test]
fn under_conflate_the_first_finishes_and_only_the_newest_of_the_rest_runs() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);
        let (mut third, c) = search(3);

        bench.cx().task().conflate().spawn(a);
        bench.cx().task().conflate().spawn(b);
        bench.cx().task().conflate().spawn(c);
        bench.run();
        assert!(first.started(), "seed {seed}");
        assert!(!third.started(), "the third waits for the first, seed {seed}");

        first.open();
        bench.run();
        assert!(third.started(), "and starts once it is done, seed {seed}");

        second.open();
        third.open();
        bench.run();

        assert!(!second.started(), "the one between never started, seed {seed}");
        assert_eq!(bench.found(), [1, 3], "seed {seed}");
        assert_eq!(bench.ends(), ["skipped", "settled", "settled"], "seed {seed}");
    }
}

#[test]
fn under_exhaust_what_comes_while_the_lane_is_busy_is_never_polled() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);
        let (mut third, c) = search(3);

        bench.cx().task().exhaust().spawn(a);
        bench.cx().task().exhaust().spawn(b);
        first.open();
        second.open();
        bench.run();

        assert!(!second.started(), "seed {seed}");
        assert_eq!(bench.found(), [1], "seed {seed}");

        bench.cx().task().exhaust().spawn(c);
        third.open();
        bench.run();

        assert_eq!(bench.found(), [1, 3], "a free lane takes work again, seed {seed}");
        assert_eq!(bench.ends(), ["skipped", "settled", "settled"], "seed {seed}");
    }
}

#[test]
fn work_held_back_is_made_when_its_turn_comes() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let made = Rc::new(RefCell::new(false));

        bench.cx().task().conflate().spawn(a);
        let noted = made.clone();
        bench.cx().task().conflate().spawn_with(move |_cancel| {
            *noted.borrow_mut() = true;
            async { Found(2) }
        });
        bench.run();
        assert!(!*made.borrow(), "made before its turn, seed {seed}");

        first.open();
        bench.run();

        assert!(*made.borrow(), "seed {seed}");
        assert_eq!(bench.found(), [1, 2], "seed {seed}");
    }
}

#[test]
fn cancelling_a_lane_ends_what_runs_and_what_waits_and_leaves_it_open() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);
        let (mut third, c) = search(3);

        bench.cx().task().conflate().spawn(a);
        bench.cx().task().conflate().spawn(b);
        bench.run();

        bench.cx().task().lane::<Found>().cancel();
        first.open();
        second.open();
        bench.run();

        assert!(bench.found().is_empty(), "seed {seed}");
        assert_eq!(bench.ends(), ["cancelled", "cancelled"], "seed {seed}");

        bench.cx().task().conflate().spawn(c);
        third.open();
        bench.run();

        assert_eq!(bench.found(), [3], "seed {seed}");
    }
}

#[test]
fn a_key_splits_a_lane() {
    for seed in SEEDS {
        let bench = Bench::new(seed);
        let (mut first, a) = search(1);
        let (mut second, b) = search(2);
        let (mut third, c) = search(3);

        bench.cx().task().latest().keyed("left").spawn(a);
        bench.cx().task().latest().keyed("right").spawn(b);
        bench.cx().task().latest().keyed("left").spawn(c);
        first.open();
        second.open();
        third.open();
        bench.run();

        let mut found = bench.found();
        found.sort_unstable();
        assert_eq!(found, [2, 3], "seed {seed}");
    }
}

#[test]
fn cancelling_under_a_key_leaves_the_other_keys_alone() {
    let bench = Bench::new(0);
    let (mut left, a) = search(1);
    let (mut right, b) = search(2);

    bench.cx().task().conflate().keyed("left").spawn(a);
    bench.cx().task().conflate().keyed("right").spawn(b);
    bench.run();

    bench.cx().task().lane::<Found>().keyed("left").cancel();
    left.open();
    right.open();
    bench.run();

    assert_eq!(bench.found(), [2]);
}

#[test]
fn a_named_lane_holds_detached_work() {
    let bench = Bench::new(0);
    let (mut first, a) = gated(());
    let (mut second, b) = gated(());

    bench.cx().task().exhaust().detached().lane::<Saving>().spawn(a);
    bench.cx().task().exhaust().detached().lane::<Saving>().spawn(b);
    first.open();
    second.open();
    bench.run();

    assert!(!second.started());
    assert_eq!(bench.ends(), ["skipped", "settled"]);
}

#[test]
fn detached_work_under_a_policy_without_a_lane_is_a_mistake_said_out_loud() {
    let bench = Bench::new(0);
    let (_gate, work) = gated(());

    let spawned = catch_unwind(AssertUnwindSafe(|| bench.cx().task().latest().detached().spawn(work)));

    let text = panic_text(spawned.expect_err("it spawned"));
    assert!(text.contains(".lane::<"), "{text}");
}

#[test]
fn one_lane_under_two_policies_is_a_mistake_that_names_both_places() {
    let bench = Bench::new(0);
    let (_first, a) = search(1);
    let (_second, b) = search(2);

    bench.cx().task().latest().spawn(a);
    let spawned = catch_unwind(AssertUnwindSafe(|| bench.cx().task().conflate().spawn(b)));

    let text = panic_text(spawned.expect_err("it spawned"));
    assert!(text.contains("Latest") && text.contains("Conflate"), "{text}");
    assert_eq!(text.matches(file!()).count(), 2, "{text}");
}

#[test]
fn cancelling_a_lane_never_named_is_a_mistake_said_out_loud() {
    let bench = Bench::new(0);

    let cancelled = catch_unwind(AssertUnwindSafe(|| bench.cx().task().cancel()));

    let text = panic_text(cancelled.expect_err("it cancelled"));
    assert!(text.contains(".lane::<"), "{text}");
}

#[test]
fn work_that_listens_for_the_token_is_left_to_wind_itself_down() {
    let bench = Bench::new(0);
    let wound_down = Arc::new(AtomicBool::new(false));

    let noted = wound_down.clone();
    bench.cx().task().spawn_with(move |gone| async move {
        gone.cancelled().await;
        executor::yield_now().await;
        noted.store(true, Ordering::SeqCst);
        Found(1)
    });
    bench.run();

    bench.home.tree.remove();
    bench.run();

    assert!(wound_down.load(Ordering::SeqCst), "the await after the cancellation still ran");
    assert!(bench.found().is_empty(), "and what it returned was for nobody");
    assert_eq!(bench.ends(), ["cancelled"]);
}

#[test]
fn the_actor_going_drops_what_waits_in_its_lanes() {
    let bench = Bench::new(0);
    let (_first, a) = search(1);
    let (second, b) = search(2);

    bench.cx().task().conflate().spawn(a);
    bench.cx().task().conflate().spawn(b);
    bench.run();

    bench.home.tree.remove();
    bench.run();

    assert!(!second.started());
    assert_eq!(bench.ends(), ["cancelled", "cancelled"]);
}
