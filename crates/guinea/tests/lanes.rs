//! Background work under a policy: which piece answers, which is cut off,
//! which never runs - and how each one ends in the trace.
//!
//! Each piece of work is held at its first await until the test opens it, so
//! the test decides what is in flight when the next one comes.

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use guinea::app::{Act, Harness};
use guinea::core::__private::tokio::sync::oneshot;
use guinea::core::actor::Cancel;
use guinea::core::actor::ctx::task::{Policy, Task};
use guinea::prelude::*;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Found(pub Vec<u32>);

#[reducer]
fn finding(this: &mut Found, n: u32) {
    this.0.push(n);
}

pub struct Answer(u32);

pub struct Saving;

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub enum Work {
    Dropped(Boxed<Answer>),
    Itself(Box<dyn FnOnce(Cancel) -> Boxed<Answer>>),
}

#[derive(Clone, Copy)]
pub enum Under {
    Free,
    Latest,
    Conflate,
    Exhaust,
}

pub struct Spawn {
    under: Under,
    key: Option<&'static str>,
    work: Work,
}

pub struct Save {
    under: Under,
    named: bool,
    work: Boxed<()>,
}

pub struct CancelLane {
    named: bool,
    key: Option<&'static str>,
}

#[derive(Debug)]
pub struct Searcher {
    push: Push<Found>,
}

actor! {
    Searcher {
        handlers { Spawn, Save, CancelLane, Answer }
    }
}

#[track_caller]
fn start<P: Policy>(task: Task<'_, Searcher, Spawn, P>, key: Option<&'static str>, work: Work) {
    let task = match key {
        Some(key) => task.keyed(key),
        None => task,
    };

    match work {
        Work::Dropped(fut) => task.spawn(fut),
        Work::Itself(make) => task.spawn_with(make),
    }
}

#[track_caller]
fn save<P: Policy>(task: Task<'_, Searcher, Save, P>, named: bool, work: Boxed<()>) {
    let task = task.detached();
    let task = match named {
        true => task.lane::<Saving>(),
        false => task,
    };

    task.spawn(work);
}

#[handler]
fn spawning(_this: &mut Searcher, Spawn { under, key, work }: Spawn, cx: Cx) {
    let task = cx.task();

    match under {
        Under::Free => start(task, key, work),
        Under::Latest => start(task.latest(), key, work),
        Under::Conflate => start(task.conflate(), key, work),
        Under::Exhaust => start(task.exhaust(), key, work),
    }
}

#[handler]
fn saving(_this: &mut Searcher, Save { under, named, work }: Save, cx: Cx) {
    let task = cx.task();

    match under {
        Under::Free => save(task, named, work),
        Under::Latest => save(task.latest(), named, work),
        Under::Conflate => save(task.conflate(), named, work),
        Under::Exhaust => save(task.exhaust(), named, work),
    }
}

#[handler]
fn cancel_lane(_this: &mut Searcher, CancelLane { named, key }: CancelLane, cx: Cx) {
    let task = cx.task();
    let task = match named {
        true => task.lane::<Answer>(),
        false => task,
    };
    let task = match key {
        Some(key) => task.keyed(key),
        None => task,
    };

    task.cancel();
}

#[handler]
fn answer(this: &mut Searcher, Answer(n): Answer) {
    this.push.send(n);
}

feature! {
    pub Lanes {
        exports { Found }
    }
}

#[installs]
fn lanes(cx: &FeatureInitContext) -> anyhow::Result<Lanes> {
    let (found, _) = cx.state::<Found>().driven_by(|push| Searcher { push });
    Ok(Lanes(found))
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

fn gated<T: Send + 'static>(answer: T) -> (Gate, Boxed<T>) {
    let (open, opened) = oneshot::channel::<()>();
    let started = Arc::new(AtomicBool::new(false));

    let gate = Gate {
        open: Some(open),
        started: started.clone(),
    };
    let work = Box::pin(async move {
        started.store(true, Ordering::SeqCst);
        let _ = opened.await;
        answer
    });

    (gate, work)
}

fn search(n: u32) -> (Gate, Work) {
    let (gate, work) = gated(Answer(n));
    (gate, Work::Dropped(work))
}

fn spawn(under: Under, work: Work) -> Spawn {
    Spawn {
        under,
        key: None,
        work,
    }
}

fn keyed(under: Under, key: &'static str, work: Work) -> Spawn {
    Spawn {
        under,
        key: Some(key),
        work,
    }
}

fn found(h: &Harness) -> Vec<u32> {
    h.state::<Found>().0.clone()
}

fn sorted(mut found: Vec<u32>) -> Vec<u32> {
    found.sort_unstable();
    found
}

/// How each piece the acts spawned ended, act by act.
fn ends(acts: &[&Act<'_>]) -> Vec<&'static str> {
    acts.iter()
        .flat_map(|act| {
            act.chain()
                .points()
                .into_iter()
                .map(|point| point.kind())
                .filter(|kind| ["settled", "cancelled", "superseded", "skipped"].contains(kind))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .unwrap_or_default(),
    }
}

#[guinea::test(iterations = 8)]
fn without_a_policy_every_piece_answers(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let (mut second, b) = search(2);

    let a = h.act::<Found>(spawn(Under::Free, a));
    let b = h.act::<Found>(spawn(Under::Free, b));
    first.open();
    second.open();
    h.settled();

    assert_eq!(sorted(found(h)), [1, 2]);
    assert_eq!(ends(&[&a, &b]), ["settled", "settled"]);
}

#[guinea::test(iterations = 8)]
fn under_latest_the_piece_in_flight_is_cut_off(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let (mut second, b) = search(2);

    let a = h.act::<Found>(spawn(Under::Latest, a));
    h.settled();
    let b = h.act::<Found>(spawn(Under::Latest, b));
    h.settled();

    first.open();
    second.open();
    h.settled();

    assert_eq!(found(h), [2]);
    assert_eq!(ends(&[&a, &b]), ["superseded", "settled"]);
}

#[guinea::test(iterations = 8)]
fn under_conflate_the_first_finishes_and_only_the_newest_of_the_rest_runs(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let (mut second, b) = search(2);
    let (mut third, c) = search(3);

    let a = h.act::<Found>(spawn(Under::Conflate, a));
    let b = h.act::<Found>(spawn(Under::Conflate, b));
    let c = h.act::<Found>(spawn(Under::Conflate, c));
    h.settled();
    assert!(first.started());
    assert!(!third.started(), "the third waits for the first");

    first.open();
    h.settled();
    assert!(third.started(), "and starts once it is done");

    second.open();
    third.open();
    h.settled();

    assert!(!second.started(), "the one between never started");
    assert_eq!(found(h), [1, 3]);
    assert_eq!(ends(&[&a, &b, &c]), ["settled", "skipped", "settled"]);
}

#[guinea::test(iterations = 8)]
fn under_exhaust_what_comes_while_the_lane_is_busy_is_never_polled(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let (mut second, b) = search(2);
    let (mut third, c) = search(3);

    let a = h.act::<Found>(spawn(Under::Exhaust, a));
    let b = h.act::<Found>(spawn(Under::Exhaust, b));
    first.open();
    second.open();
    h.settled();

    assert!(!second.started());
    assert_eq!(found(h), [1]);

    let c = h.act::<Found>(spawn(Under::Exhaust, c));
    third.open();
    h.settled();

    assert_eq!(found(h), [1, 3], "a free lane takes work again");
    assert_eq!(ends(&[&a, &b, &c]), ["settled", "skipped", "settled"]);
}

#[guinea::test(iterations = 8)]
fn work_held_back_is_made_when_its_turn_comes(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let made = Arc::new(AtomicBool::new(false));

    h.act::<Found>(spawn(Under::Conflate, a));
    let noted = made.clone();
    let b = Work::Itself(Box::new(move |_cancel| {
        noted.store(true, Ordering::SeqCst);
        Box::pin(async { Answer(2) })
    }));
    h.act::<Found>(spawn(Under::Conflate, b));
    h.settled();
    assert!(!made.load(Ordering::SeqCst), "made before its turn");

    first.open();
    h.settled();

    assert!(made.load(Ordering::SeqCst));
    assert_eq!(found(h), [1, 2]);
}

#[guinea::test(iterations = 8)]
fn cancelling_a_lane_ends_what_runs_and_what_waits_and_leaves_it_open(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let (mut second, b) = search(2);
    let (mut third, c) = search(3);

    let a = h.act::<Found>(spawn(Under::Conflate, a));
    let b = h.act::<Found>(spawn(Under::Conflate, b));
    h.settled();

    h.act::<Found>(CancelLane {
        named: true,
        key: None,
    });
    first.open();
    second.open();
    h.settled();

    assert!(found(h).is_empty());
    assert_eq!(ends(&[&a, &b]), ["cancelled", "cancelled"]);

    h.act::<Found>(spawn(Under::Conflate, c));
    third.open();
    h.settled();

    assert_eq!(found(h), [3]);
}

#[guinea::test(iterations = 8)]
fn a_key_splits_a_lane(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = search(1);
    let (mut second, b) = search(2);
    let (mut third, c) = search(3);

    h.act::<Found>(keyed(Under::Latest, "left", a));
    h.act::<Found>(keyed(Under::Latest, "right", b));
    h.act::<Found>(keyed(Under::Latest, "left", c));
    first.open();
    second.open();
    third.open();
    h.settled();

    assert_eq!(sorted(found(h)), [2, 3]);
}

#[guinea::test(iterations = 8)]
fn cancelling_under_a_key_leaves_the_other_keys_alone(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut left, a) = search(1);
    let (mut right, b) = search(2);

    h.act::<Found>(keyed(Under::Conflate, "left", a));
    h.act::<Found>(keyed(Under::Conflate, "right", b));
    h.settled();

    h.act::<Found>(CancelLane {
        named: true,
        key: Some("left"),
    });
    left.open();
    right.open();
    h.settled();

    assert_eq!(found(h), [2]);
}

#[guinea::test(iterations = 8)]
fn a_named_lane_holds_detached_work(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (mut first, a) = gated(());
    let (mut second, b) = gated(());

    let a = h.act::<Found>(Save {
        under: Under::Exhaust,
        named: true,
        work: a,
    });
    let b = h.act::<Found>(Save {
        under: Under::Exhaust,
        named: true,
        work: b,
    });
    first.open();
    second.open();
    h.settled();

    assert!(!second.started());
    assert_eq!(ends(&[&a, &b]), ["settled", "skipped"]);
}

#[guinea::test(iterations = 8)]
fn detached_work_under_a_policy_without_a_lane_is_a_mistake_said_out_loud(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (_gate, work) = gated(());

    let spawned = catch_unwind(AssertUnwindSafe(|| {
        h.dispatch::<Found>().emit(Save {
            under: Under::Latest,
            named: false,
            work,
        })
    }));

    let text = panic_text(spawned.expect_err("it spawned"));
    assert!(text.contains(".lane::<"), "{text}");
}

#[guinea::test(iterations = 8)]
fn one_lane_under_two_policies_is_a_mistake_that_names_both_places(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();
    let (_first, a) = search(1);
    let (_second, b) = search(2);

    h.act::<Found>(spawn(Under::Latest, a));
    let spawned = catch_unwind(AssertUnwindSafe(|| {
        h.dispatch::<Found>().emit(spawn(Under::Conflate, b))
    }));

    let text = panic_text(spawned.expect_err("it spawned"));
    assert!(text.contains("Latest") && text.contains("Conflate"), "{text}");
    assert_eq!(text.matches(file!()).count(), 2, "{text}");
}

#[guinea::test(iterations = 8)]
fn cancelling_a_lane_never_named_is_a_mistake_said_out_loud(h: &mut Harness) {
    h.install::<Lanes>(&()).unwrap();

    let cancelled = catch_unwind(AssertUnwindSafe(|| {
        h.dispatch::<Found>().emit(CancelLane {
            named: false,
            key: None,
        })
    }));

    let text = panic_text(cancelled.expect_err("it cancelled"));
    assert!(text.contains(".lane::<"), "{text}");
}

#[guinea::test(iterations = 8)]
fn work_that_listens_for_the_token_is_left_to_wind_itself_down(h: &mut Harness) {
    let page = h.child();
    page.install::<Lanes>(&()).unwrap();
    let wound_down = Arc::new(AtomicBool::new(false));

    let noted = wound_down.clone();
    let work = Work::Itself(Box::new(move |gone: Cancel| {
        Box::pin(async move {
            gone.cancelled().await;
            guinea::core::executor::yield_now().await;
            noted.store(true, Ordering::SeqCst);
            Answer(1)
        })
    }));
    let act = page.act::<Found>(spawn(Under::Free, work));
    h.settled();

    page.leave();
    h.settled();

    assert!(wound_down.load(Ordering::SeqCst), "the await after the cancellation still ran");
    assert!(!act.chain().handled::<Answer>(), "and what it returned was for nobody");
    assert_eq!(ends(&[&act]), ["cancelled"]);
}

#[guinea::test(iterations = 8)]
fn the_actor_going_drops_what_waits_in_its_lanes(h: &mut Harness) {
    let page = h.child();
    page.install::<Lanes>(&()).unwrap();
    let (_first, a) = search(1);
    let (second, b) = search(2);

    let a = page.act::<Found>(spawn(Under::Conflate, a));
    let b = page.act::<Found>(spawn(Under::Conflate, b));
    h.settled();

    page.leave();
    h.settled();

    assert!(!second.started());
    assert_eq!(ends(&[&a, &b]), ["cancelled", "cancelled"]);
}
