use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

use super::*;
use crate::actor::addr::{Addr, TestHome};
use crate::actor::{Cx, Handler};
use crate::executor::{Installed, install};
use crate::scope::ScopeTree;

/// The test's own clock: ticks wait on it, and it moves only when told.
fn clock() -> Installed {
    install(0)
}

fn wait(clock: &Installed, ms: u64) {
    clock.advance(Duration::from_millis(ms));
}

fn count(counter: &Arc<AtomicUsize>) -> usize {
    counter.load(Ordering::SeqCst)
}

#[track_caller]
fn counting(tree: &ScopeTree, period: impl Into<Period>) -> (Source, Arc<AtomicUsize>) {
    let counter = Arc::new(AtomicUsize::new(0));

    let c = counter.clone();
    let source = tree.section().repeat(period, move || {
        c.fetch_add(1, Ordering::SeqCst);
    });

    (source, counter)
}

#[track_caller]
fn every(tree: &ScopeTree, ms: u64) -> (Source, Arc<AtomicUsize>) {
    counting(tree, Duration::from_millis(ms))
}

fn watched(run: impl FnOnce()) -> Vec<Change> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    changes::watch(move |change| sink.borrow_mut().push(change.clone()));

    run();
    changes::stop_watching();

    seen.take()
}

/// A stream fed by hand, from any thread.
#[derive(Clone, Default)]
struct Pipe(Arc<Mutex<(VecDeque<u32>, Option<Waker>, bool)>>);

impl Pipe {
    fn send(&self, item: u32) {
        let waker = {
            let mut pipe = self.0.lock();
            pipe.0.push_back(item);
            pipe.1.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn end(&self) {
        let waker = {
            let mut pipe = self.0.lock();
            pipe.2 = true;
            pipe.1.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl Stream for Pipe {
    type Item = u32;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<u32>> {
        let mut pipe = self.0.lock();

        if let Some(item) = pipe.0.pop_front() {
            return Poll::Ready(Some(item));
        }
        if pipe.2 {
            return Poll::Ready(None);
        }

        pipe.1 = Some(cx.waker().clone());
        Poll::Pending
    }
}

fn piped(tree: &ScopeTree) -> (Pipe, Rc<RefCell<Vec<u32>>>) {
    let pipe = Pipe::default();
    let heard = Rc::new(RefCell::new(Vec::new()));

    let into = heard.clone();
    tree.section()
        .pour(pipe.clone(), move |item| into.borrow_mut().push(item));

    (pipe, heard)
}

#[test]
fn devtools_hear_when_a_source_opens_and_when_it_closes() {
    let _clock = clock();
    let tree = ScopeTree::new();
    let mut id = None;

    let seen = watched(|| {
        let (source, _) = every(&tree, 30);
        id = source.id();
        tree.remove();
    });

    let id = id.expect("open");
    assert_eq!(seen, [Change::SourceOpened { id }, Change::SourceClosed { id }]);
}

#[test]
fn an_untraced_source_leaves_devtools_at_once_and_ends_unheard() {
    let _clock = clock();
    let tree = ScopeTree::new();
    let mut id = None;

    let seen = watched(|| {
        let (source, _) = every(&tree, 30);
        id = source.id();
        let _source = source.untraced();
        tree.remove();
    });

    let id = id.expect("open");
    assert_eq!(seen, [Change::SourceOpened { id }, Change::SourceClosed { id }]);
}

#[test]
fn a_timer_ticks_every_period() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (_source, counter) = every(&tree, 30);

    wait(&clock, 29);
    assert_eq!(count(&counter), 0, "ticked before its period was up");

    wait(&clock, 1);
    assert_eq!(count(&counter), 1);

    wait(&clock, 30);
    assert_eq!(count(&counter), 2);
}

#[test]
fn an_item_says_which_source_it_came_from() {
    let clock = clock();
    let tree = ScopeTree::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let heard = seen.clone();
    crate::trace::observe(move |trace| {
        let record = match trace {
            crate::trace::Trace::Begin(record) | crate::trace::Trace::Mark(record) => record,
            _ => return,
        };
        if matches!(record.point, Point::Source { .. } | Point::Arrived { .. }) {
            heard.borrow_mut().push(record.point.clone());
        }
    });

    let place = Location::caller();
    let (source, _) = every(&tree, 30);
    let source = source.named("flush");
    wait(&clock, 30);
    crate::trace::stop_observing();

    let id = source.id().expect("open");
    assert_eq!(
        *seen.borrow(),
        [
            Point::Source {
                source: id,
                to: "fn",
                output: "()",
                file: place.file(),
                line: place.line() + 1,
            },
            Point::Arrived {
                source: id,
                name: Some("flush"),
                to: "fn",
                output: "()",
            },
        ]
    );
}

#[test]
fn a_timer_stops_with_its_scope() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (_source, counter) = every(&tree, 30);

    wait(&clock, 30);
    assert_eq!(count(&counter), 1);
    tree.remove();

    wait(&clock, 90);
    assert_eq!(count(&counter), 1);
}

#[test]
fn a_source_gone_with_its_scope_leaves_nothing_waiting() {
    let clock = clock();
    let tree = ScopeTree::new();
    tree.section().repeat(Duration::from_secs(60), || {});
    clock.run_until_parked();
    assert_eq!(clock.stuck(), 1, "its wait is under way");

    tree.remove();
    clock.run_until_parked();

    assert_eq!(clock.stuck(), 0, "the wait outlived the source it was for");
}

#[test]
fn a_timer_gone_before_its_first_tick_never_ticks() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (_source, counter) = every(&tree, 200);

    tree.remove();

    wait(&clock, 250);
    assert_eq!(count(&counter), 0);
}

#[test]
fn a_timer_sleeps_with_its_scope_and_ticks_once_when_it_wakes() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (_source, counter) = every(&tree, 30);

    wait(&clock, 30);
    assert_eq!(count(&counter), 1);

    tree.sleep();
    wait(&clock, 90);
    assert_eq!(count(&counter), 1, "ticked while its scope slept");

    tree.wake();
    clock.run_until_parked();
    assert_eq!(count(&counter), 2, "the ticks it slept through come as one, at the wake");

    wait(&clock, 30);
    assert_eq!(count(&counter), 3, "and the next a period after it");
}

#[test]
fn a_source_is_not_pulled_while_its_scope_sleeps() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (pipe, heard) = piped(&tree);

    pipe.send(1);
    clock.run_until_parked();
    assert_eq!(*heard.borrow(), [1]);

    tree.sleep();
    pipe.send(2);
    pipe.send(3);
    clock.run_until_parked();
    assert_eq!(*heard.borrow(), [1], "poured into a scope that sleeps");

    tree.wake();
    clock.run_until_parked();
    assert_eq!(*heard.borrow(), [1, 2, 3], "what the stream kept comes at the wake, in order");
}

#[test]
fn an_item_pulled_before_its_scope_fell_asleep_waits_for_the_wake() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (pipe, heard) = piped(&tree);
    clock.run_until_parked();

    pipe.send(1);
    assert!(clock.step(), "the source pulled the item and sent it on");
    tree.sleep();
    clock.run_until_parked();
    assert!(heard.borrow().is_empty(), "handed to a scope that sleeps");

    tree.wake();
    clock.run_until_parked();
    assert_eq!(*heard.borrow(), [1]);
}

#[test]
fn a_source_that_runs_dry_lets_itself_go() {
    let clock = clock();
    let tree = ScopeTree::new();
    let (pipe, heard) = piped(&tree);

    pipe.send(1);
    pipe.end();
    clock.run_until_parked();

    assert_eq!(*heard.borrow(), [1]);
    assert!(tree.scope().sources().is_empty(), "still listed once dry");
    assert_eq!(clock.stuck(), 0);
}

#[test]
fn a_single_tick_comes_once() {
    let clock = clock();
    let tree = ScopeTree::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let c = counter.clone();
    tree.section().pour(Ticks::once(Duration::from_millis(30)), move |()| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    wait(&clock, 90);
    assert_eq!(count(&counter), 1);
    assert!(tree.scope().sources().is_empty());
}

#[test]
#[should_panic(expected = "opening a source")]
fn a_source_for_a_scope_that_is_gone_is_a_mistake() {
    let _clock = clock();
    let tree = ScopeTree::new();
    let section = tree.section();
    tree.remove();

    section.repeat(Duration::from_millis(30), || {});
}

type Watcher = (u64, Arc<dyn Fn() + Send + Sync>);

/// A value set by hand, telling whoever watches it, from the thread that
/// set it.
#[derive(Clone, Default)]
struct Knob(Arc<std::sync::Mutex<(Option<u64>, Vec<Watcher>, u64)>>);

impl Knob {
    fn set(&self, ms: u64) {
        let watchers: Vec<Watcher> = {
            let mut knob = self.0.lock().unwrap();
            knob.0 = Some(ms);
            knob.1.clone()
        };
        for (_, changed) in watchers {
            changed();
        }
    }

    fn watchers(&self) -> usize {
        self.0.lock().unwrap().1.len()
    }
}

struct Unwatch(Knob, u64);

impl Drop for Unwatch {
    fn drop(&mut self) {
        let id = self.1;
        self.0.0.lock().unwrap().1.retain(|(watcher, _)| *watcher != id);
    }
}

impl Changing for Knob {
    type Value = u64;

    fn now(&self) -> Option<u64> {
        self.0.lock().unwrap().0
    }

    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        let mut knob = self.0.lock().unwrap();
        knob.2 += 1;
        let id = knob.2;
        knob.1.push((id, Arc::from(changed)));
        Box::new(Unwatch(self.clone(), id))
    }
}

fn following(tree: &ScopeTree, knob: &Knob) -> (Source, Arc<AtomicUsize>) {
    counting(tree, Period::follows(knob.clone(), Duration::from_millis))
}

#[test]
fn a_period_that_follows_a_value_changes_the_moment_the_value_does() {
    let clock = clock();
    let tree = ScopeTree::new();
    let knob = Knob::default();
    knob.set(1_000);
    let (_source, counter) = following(&tree, &knob);

    wait(&clock, 100);
    knob.set(50);

    wait(&clock, 50);
    assert_eq!(count(&counter), 1, "the second it had left was waited out");

    wait(&clock, 50);
    assert_eq!(count(&counter), 2);
}

#[test]
fn a_change_leaves_only_the_new_wait_waiting() {
    let clock = clock();
    let tree = ScopeTree::new();
    let knob = Knob::default();
    knob.set(60_000);
    let (_source, _counter) = following(&tree, &knob);
    clock.run_until_parked();

    knob.set(50);
    clock.run_until_parked();

    assert_eq!(clock.stuck(), 1, "the minute's wait went on beside the new one");
}

#[test]
fn a_period_with_nothing_to_follow_waits_for_something() {
    let clock = clock();
    let tree = ScopeTree::new();
    let knob = Knob::default();
    let (_source, counter) = following(&tree, &knob);

    wait(&clock, 10_000);
    assert_eq!(count(&counter), 0, "ticked with no period to tick by");

    knob.set(30);
    wait(&clock, 30);
    assert_eq!(count(&counter), 1);
}

#[test]
fn what_a_period_follows_is_watched_for_as_long_as_the_timer_runs() {
    let clock = clock();
    let tree = ScopeTree::new();
    let knob = Knob::default();
    knob.set(1_000);

    let (_source, _counter) = following(&tree, &knob);
    assert_eq!(knob.watchers(), 1);

    tree.remove();
    clock.run_until_parked();
    assert_eq!(knob.watchers(), 0, "the timer stopped and kept watching");
}

#[test]
fn an_item_may_list_the_sources_itself() {
    let clock = clock();
    let tree = ScopeTree::new();
    let seen = Arc::new(AtomicUsize::new(0));

    let s = seen.clone();
    let scope = tree.scope();
    tree.section().repeat(Duration::from_millis(30), move || {
        s.store(scope.sources().len(), Ordering::SeqCst);
    });

    wait(&clock, 30);
    assert_eq!(count(&seen), 1);
}

#[test]
fn an_item_may_open_another_source() {
    let clock = clock();
    let tree = ScopeTree::new();
    let section = tree.section();

    tree.section().repeat(Duration::from_millis(30), move || {
        section.repeat(Duration::from_secs(60), || {});
    });

    wait(&clock, 30);
    assert_eq!(tree.scope().sources().len(), 2);
}

#[test]
fn an_open_source_says_where_it_was_opened_whom_it_pours_into_and_what_it_is_called() {
    let _clock = clock();
    let tree = ScopeTree::new();
    let page = tree.child();
    let (source, _) = every(&tree, 1_000);
    let (hidden, _) = every(&tree, 1_000);
    let below = page.section().repeat(Duration::from_millis(1_000), || {});

    let source = source.named("sweep");
    let hidden = hidden.untraced().id();

    let id = source.id().expect("open");
    let listed = tree.scope().sources();
    let info = listed.iter().find(|info| info.id == id).expect("listed");
    assert_eq!(info.name, Some("sweep"));
    assert!(info.place.file().ends_with("tests.rs"));
    assert_eq!((info.to, info.output), ("fn", "()"));
    assert_eq!(info.scope, tree.scope());
    assert!(listed.iter().all(|info| Some(info.id) != hidden));
    assert!(listed.iter().any(|info| Some(info.id) == below.id()), "a source below is listed too");
    assert!(tree.scope().sources_here().iter().all(|info| Some(info.id) != below.id()));

    tree.remove();
    assert_eq!(source.id(), None);
}

struct Watch;
struct Heard(u32, u32);

#[derive(Debug)]
struct Listener {
    heard: Rc<RefCell<Vec<(u32, u32)>>>,
}

guinea_macros::actor! {
    Listener {
        handlers {
            Watch => { task Heard }
            Heard
        }
    }
}

impl Handler<Watch> for Listener {
    fn handle(&mut self, _: Watch, _cx: Cx<Self, Watch>) {}
}

impl Handler<Heard> for Listener {
    fn handle(&mut self, Heard(which, item): Heard, _cx: Cx<Self, Heard>) {
        self.heard.borrow_mut().push((which, item));
    }
}

struct Listening {
    home: TestHome,
    addr: Addr<Listener>,
    heard: Rc<RefCell<Vec<(u32, u32)>>>,
}

impl Listening {
    fn new() -> Self {
        let home = TestHome::new();
        let heard = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new(Listener { heard: heard.clone() }, &home);

        Self { home, addr, heard }
    }

    fn cx(&self) -> Cx<Listener, Watch> {
        Cx::new(self.addr.clone())
    }

    fn heard(&self) -> Vec<(u32, u32)> {
        self.heard.borrow().clone()
    }
}

#[test]
fn an_actors_source_is_held_by_its_scope_and_sleeps_with_it() {
    let clock = clock();
    let listening = Listening::new();
    let pipe = Pipe::default();

    listening.cx().task().pour(pipe.clone(), |item| Heard(1, item));
    assert_eq!(listening.home.tree.scope().sources().len(), 1);

    listening.home.tree.sleep();
    pipe.send(7);
    clock.run_until_parked();
    assert!(listening.heard().is_empty(), "poured into an actor whose scope sleeps");

    listening.home.tree.wake();
    clock.run_until_parked();
    assert_eq!(listening.heard(), [(1, 7)]);
}

#[test]
fn under_latest_a_new_source_replaces_the_one_that_pours() {
    let clock = clock();
    let listening = Listening::new();
    let (old, new) = (Pipe::default(), Pipe::default());

    listening.cx().task().latest().pour(old.clone(), |item| Heard(1, item));
    old.send(1);
    clock.run_until_parked();

    listening.cx().task().latest().pour(new.clone(), |item| Heard(2, item));
    old.send(2);
    new.send(3);
    clock.run_until_parked();

    assert_eq!(listening.heard(), [(1, 1), (2, 3)]);
    assert_eq!(listening.home.tree.scope().sources().len(), 1, "the old one is still held");
}

#[test]
fn under_exhaust_a_source_is_turned_away_while_one_pours_and_taken_once_it_ran_dry() {
    let clock = clock();
    let listening = Listening::new();
    let (first, second, third) = (Pipe::default(), Pipe::default(), Pipe::default());

    listening.cx().task().exhaust().pour(first.clone(), |item| Heard(1, item));
    listening.cx().task().exhaust().pour(second.clone(), |item| Heard(2, item));
    second.send(1);
    first.end();
    clock.run_until_parked();

    listening.cx().task().exhaust().pour(third.clone(), |item| Heard(3, item));
    third.send(2);
    clock.run_until_parked();

    assert_eq!(listening.heard(), [(3, 2)]);
}
