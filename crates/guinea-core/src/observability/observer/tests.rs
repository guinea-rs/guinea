use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::*;
use crate::actor::event_bus::HeardBy;
use crate::actor::Addr;
use crate::actor::addr::Home;
use crate::scope::ScopeTree;
use crate::trace::{self, Point};

#[derive(Default, Clone, PartialEq, Debug)]
struct Count(u32);

#[guinea_macros::reducer]
fn count(this: &mut Count, by: u32) {
    this.0 += by;
}

#[derive(Debug)]
struct Ticker(u32);

struct Tick;

guinea_macros::actor! {
    Ticker {
        handlers {
            Tick
        }
    }
}

impl crate::actor::Handler<Tick> for Ticker {
    fn handle(crate::actor::In(this, _, _): crate::actor::In<Self, Tick>) {
        this.0 += 1;
    }
}

#[derive(Clone)]
struct Ping;

impl crate::actor::event_bus::Event for Ping {}

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// One tool watches the process, so the tests that watch take turns.
fn turn() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Watched {
    watching: Watching,
    _turn: MutexGuard<'static, ()>,
}

impl std::ops::Deref for Watched {
    type Target = Watching;

    fn deref(&self) -> &Watching {
        &self.watching
    }
}

fn watching(scope: Scope) -> Watched {
    let turn = turn();
    let watching = watch(scope, || {}).expect("nothing else watches");
    Watched {
        watching,
        _turn: turn,
    }
}

fn counted() -> &'static str {
    std::any::type_name::<Count>()
}

#[test]
fn a_watch_starts_with_the_tree_as_it_is() {
    let tree = ScopeTree::new();
    let window = tree.child();
    let page = window.child();
    page.open_section(Some("app::Counting"), None);
    page.push::<Count>(1);
    let watched = watching(tree.scope());

    let events = watched.take().events;

    let root = tree.scope().id();
    assert_eq!(
        events,
        [
            Event::ScopeOpened { scope: root, parent: None },
            Event::ScopeOpened { scope: window.id(), parent: Some(root) },
            Event::ScopeOpened { scope: page.id(), parent: Some(window.id()) },
            Event::Installed { scope: page.id(), name: "app::Counting", declared: None },
            Event::StateMoved { scope: page.id(), reducer: counted() },
        ]
    );
}

#[test]
fn a_watch_starts_with_everything_the_tree_holds() {
    let tree = ScopeTree::new();
    let page = tree.child();
    let ticker = Addr::listed(Ticker(0), &Home::new(page), None);
    let hearing = tree.bus().subscribe_fn(|_: Ping| {});
    page.section().note_listener("Ping", None);
    page.sleep();
    let watched = watching(tree.scope());

    let events = watched.take().events;

    let told = |event: &Event| events.contains(event);
    assert!(
        told(&Event::ActorAdded {
            id: ticker.id(),
            type_name: crate::short_type_name::<Ticker>(),
            owner: Owner { scope: Some(page.id()), feature: None, drives: None },
        }) && told(&Event::Slept { scope: page.id() })
            && told(&Event::Listened {
                scope: page.id(),
                listener: Listener { event: "Ping", actor: None, feature: None },
            })
            && events.iter().any(|event| matches!(event, Event::Subscribed { by: HeardBy::Callback(_), .. })),
        "{events:#?}"
    );
    drop(hearing);
}

#[test]
fn what_moves_after_the_tree_comes_in_the_next_take() {
    let tree = ScopeTree::new();
    let watched = watching(tree.scope());
    watched.take();

    let page = tree.child();
    page.push::<Count>(2);
    page.open_section(Some("app::Late"), None).withdraw();

    assert_eq!(
        watched.take().events,
        [
            Event::ScopeOpened { scope: page.id(), parent: Some(tree.id()) },
            Event::Installed { scope: page.id(), name: "app::Late", declared: None },
            Event::Withdrawn { scope: page.id(), name: "app::Late" },
            Event::StateMoved { scope: page.id(), reducer: counted() },
        ]
    );
}

#[test]
fn a_state_or_an_actor_that_moved_many_times_is_told_once_a_batch() {
    let tree = ScopeTree::new();
    let ticker = Addr::listed(Ticker(0), &Home::new(tree.scope()), None);
    let watched = watching(tree.scope());
    watched.take();

    for _ in 0..3 {
        tree.push::<Count>(1);
        ticker.send(Tick);
    }

    assert_eq!(
        watched.take().events,
        [
            Event::ActorHandled { id: ticker.id() },
            Event::StateMoved { scope: tree.id(), reducer: counted() },
        ]
    );
}

#[test]
fn a_scope_that_goes_is_told_after_what_it_held_and_the_scopes_under_it() {
    let tree = ScopeTree::new();
    let window = tree.child();
    let page = window.child();
    let ticker = Addr::listed(Ticker(0), &Home::new(page), None);
    let watched = watching(tree.scope());
    watched.take();

    page.push::<Count>(1);
    ticker.send(Tick);
    window.remove();

    assert_eq!(
        watched.take().events,
        [
            Event::ActorRemoved { id: ticker.id() },
            Event::ScopeClosed { scope: page.id() },
            Event::ScopeClosed { scope: window.id() },
        ]
    );
}

#[test]
fn an_actor_that_comes_and_goes_is_told_with_whose_it_is() {
    let tree = ScopeTree::new();
    let page = tree.child();
    let clock = page.open_section(Some("app::Clock"), None);
    let watched = watching(tree.scope());
    watched.take();

    let id = Addr::listed(Ticker(0), &Home::in_section(clock), Some("Time")).id();
    page.remove();

    assert_eq!(
        watched.take().events,
        [
            Event::ActorAdded {
                id,
                type_name: crate::short_type_name::<Ticker>(),
                owner: Owner {
                    scope: Some(page.id()),
                    feature: Some("app::Clock"),
                    drives: Some("Time"),
                },
            },
            Event::ActorRemoved { id },
            Event::ScopeClosed { scope: page.id() },
        ]
    );
}

#[test]
fn a_scope_that_sleeps_and_wakes_is_told() {
    let tree = ScopeTree::new();
    let page = tree.child();
    let watched = watching(tree.scope());
    watched.take();

    page.sleep();
    page.wake();

    assert_eq!(
        watched.take().events,
        [Event::Slept { scope: page.id() }, Event::Woke { scope: page.id() }]
    );
}

#[test]
fn who_joins_and_leaves_the_bus_is_told() {
    let tree = ScopeTree::new();
    let watched = watching(tree.scope());
    watched.take();

    let hearing = tree.bus().subscribe_fn(|_: Ping| {});
    drop(hearing);

    let events = watched.take().events;
    assert!(
        matches!(
            events.as_slice(),
            [
                Event::Subscribed { seq, event, by: HeardBy::Callback(_), answers: false },
                Event::Unsubscribed { seq: gone },
            ] if seq == gone && event.ends_with("Ping")
        ),
        "{events:#?}"
    );
}

#[tokio::test]
async fn a_source_that_opens_and_closes_is_told() {
    let tree = ScopeTree::new();
    let watched = watching(tree.scope());
    watched.take();

    let source = tree.section().repeat(std::time::Duration::from_secs(60), || {});
    let id = source.id();
    tree.remove();

    let events = watched.take().events;
    let opened = events.iter().find_map(|event| match event {
        Event::SourceOpened(info) => Some((info.id, info.scope)),
        _ => None,
    });
    assert_eq!(
        (opened, events.last()),
        (
            id.map(|id| (id, tree.id())),
            Some(&Event::ScopeClosed { scope: tree.id() })
        ),
        "{events:#?}"
    );
    assert!(events.iter().any(|event| matches!(event, Event::SourceClosed { .. })));
}

#[tokio::test]
async fn a_source_named_after_it_opened_is_told_its_name() {
    let tree = ScopeTree::new();
    let watched = watching(tree.scope());
    watched.take();

    let source = tree
        .section()
        .repeat(std::time::Duration::from_secs(60), || {})
        .named("heartbeat");

    let events = watched.take().events;
    assert_eq!(
        events.last(),
        source.id().map(|id| Event::SourceNamed { id, name: "heartbeat" }).as_ref(),
        "{events:#?}"
    );
}

#[test]
fn another_tree_on_the_thread_is_not_told() {
    let tree = ScopeTree::new();
    let other = ScopeTree::new();
    let watched = watching(tree.scope());
    watched.take();

    let _page = other.child();
    other.push::<Count>(1);

    assert_eq!(watched.take().events, []);
}

#[test]
fn one_tool_watches_at_a_time() {
    let _turn = turn();
    let tree = ScopeTree::new();
    let other = ScopeTree::new();

    let first = watch(tree.scope(), || {}).expect("nothing else watches");
    let refused = watch(other.scope(), || {}).is_err();
    drop(first);
    let again = watch(other.scope(), || {}).is_ok();

    assert_eq!((refused, again), (true, true));
}

#[test]
fn a_tool_is_woken_once_when_something_moves_after_it_took() {
    let _turn = turn();
    let tree = ScopeTree::new();
    let woken = Arc::new(AtomicUsize::new(0));
    let counting = woken.clone();
    let watching = watch(tree.scope(), move || {
        counting.fetch_add(1, Ordering::SeqCst);
    })
    .expect("nothing else watches");
    let for_the_tree = woken.swap(0, Ordering::SeqCst);
    watching.take();

    let _first = tree.child();
    let _second = tree.child();

    assert_eq!((for_the_tree, woken.load(Ordering::SeqCst)), (1, 1));
}

#[test]
fn what_is_traced_here_and_on_other_threads_comes_in_the_batch() {
    let tree = ScopeTree::new();
    let watched = watching(tree.scope());
    watched.take();

    let here = trace::mark(|| Point::Push { reducer: "Here" });
    let (there_thread, there) =
        std::thread::spawn(|| (trace::thread_id(), trace::mark(|| Point::Push { reducer: "There" })))
            .join()
            .expect("marking thread");

    let batch = watched.take();
    let made_on = |cause| {
        batch.trace.iter().find_map(|(thread, record)| match record {
            Trace::Mark(record) if record.id == cause => Some(*thread),
            _ => None,
        })
    };
    assert_eq!(
        (made_on(here), made_on(there)),
        (Some(trace::thread_id()), Some(there_thread))
    );
}

#[test]
fn a_watch_that_ended_hears_nothing_and_lets_another_start() {
    let _turn = turn();
    let tree = ScopeTree::new();
    let watching = watch(tree.scope(), || {}).expect("nothing else watches");
    drop(watching);

    let observed = trace::is_observed();
    let again = watch(tree.scope(), || {}).map(|watching| watching.take().events.len());

    assert_eq!((observed, again.ok()), (false, Some(1)));
}
