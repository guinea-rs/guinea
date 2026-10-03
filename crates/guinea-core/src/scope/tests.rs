use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::actor::Addr;
use crate::actor::registry::Owner;

#[derive(Default, Clone, PartialEq, Debug)]
struct Counter {
    value: i32,
}

#[derive(Clone)]
enum CounterMsg {
    Set(i32),
}

impl Reducer for Counter {
    type Update = CounterMsg;

    fn reduce(&mut self, update: CounterMsg) {
        match update {
            CounterMsg::Set(v) => self.value = v,
        }
    }
}

struct Said(&'static str, Rc<RefCell<Vec<&'static str>>>);

impl Teardown for Said {
    fn teardown(self) {
        self.1.borrow_mut().push(self.0);
    }
}

#[test]
fn a_tree_takes_every_scope_in_it_along_when_its_owner_lets_go() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let tree = ScopeTree::new();
    let window = tree.child();
    let page = window.child();
    tree.own(Said("application", said.clone()));
    page.own(Said("page", said.clone()));

    drop(tree);

    assert_eq!(*said.borrow(), ["page", "application"]);
    assert!(!window.is_alive());
    assert!(!page.is_alive());
}

#[test]
fn removing_a_scope_tears_down_what_is_under_it_first() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let root = ScopeTree::new();
    let older = root.child();
    let below = older.child();
    let newer = root.child();

    for (scope, name) in [
        (root.scope(), "root"),
        (older, "older"),
        (below, "below"),
        (newer, "newer"),
    ] {
        scope.own(Said(name, said.clone()));
    }
    root.remove();

    assert_eq!(*said.borrow(), ["newer", "below", "older", "root"]);
}

#[test]
fn a_scope_lets_go_of_what_it_holds_in_the_reverse_of_the_order_it_took_it() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let scope = ScopeTree::new();
    scope.own(Said("store", said.clone()));
    scope.own(Said("actor reading the store", said.clone()));

    scope.remove();

    assert_eq!(*said.borrow(), ["actor reading the store", "store"]);
}

#[test]
fn a_removed_scope_stays_gone_and_its_slot_names_another_one() {
    let root = ScopeTree::new();
    let gone = root.child();
    gone.remove();
    let next = root.child();

    assert!(!gone.is_alive());
    assert!(next.is_alive());
    assert_ne!(gone, next);
    assert_ne!(gone.key(), next.key());
    assert_eq!(next.parent(), Some(root.scope()));
}

#[test]
fn a_scope_goes_when_it_is_removed_however_many_name_it() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let tree = ScopeTree::new();
    let scope = tree.child();
    let named_elsewhere = scope;
    scope.own(Said("torn down", said.clone()));

    scope.remove();

    assert_eq!(*said.borrow(), ["torn down"]);
    assert!(!named_elsewhere.is_alive());
}

#[test]
fn what_a_removed_scope_is_handed_is_torn_down_at_once() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let scope = ScopeTree::new();
    scope.remove();

    scope.own(Said("at once", said.clone()));

    assert_eq!(*said.borrow(), ["at once"]);
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
    fn handle(&mut self, _: Tick, _cx: crate::actor::Cx<Self, Tick>) {
        self.0 += 1;
    }
}

fn ticker() -> Addr<Ticker> {
    Addr::new_managed_scoped(
        Ticker(0),
        crate::actor::UiThreadToken::dangerously_create_token_unchecked(),
    )
}

fn watched(run: impl FnOnce()) -> Vec<crate::devtools::Change> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    crate::devtools::watch(move |change| sink.borrow_mut().push(change.clone()));

    run();
    crate::devtools::stop_watching();

    seen.take()
}

#[test]
fn devtools_hear_an_actor_a_scope_holds_come_and_go_and_whose_it_is() {
    use crate::devtools::Change;

    let window = ScopeTree::new();
    window.set_window(7);
    let page = window.child();
    let addr = ticker();
    let id = addr.id();

    let seen = watched(|| {
        page.hold_actor(&addr, Some("app::Clock"), Some("Time"));
        window.remove();
    });

    assert_eq!(
        seen,
        [
            Change::ActorAdded {
                root: Some(7),
                id,
                type_name: crate::actor::short_type_name::<Ticker>(),
                owner: Owner {
                    scope: Some(page.key()),
                    feature: Some("app::Clock"),
                    drives: Some("Time"),
                },
            },
            Change::ActorRemoved { root: Some(7), id },
        ]
    );
}

#[test]
fn a_window_reads_the_actors_under_it_and_one_by_id_without_the_rest() {
    let window = ScopeTree::new();
    let page = window.child();
    let (bumped, other) = (ticker(), ticker());
    window.hold_actor(&other, None, None);
    page.hold_actor(&bumped, None, None);
    bumped.send(Tick);

    let mut held: Vec<usize> = window.actors().iter().map(|actor| actor.id).collect();
    held.sort_unstable();
    let mut expected = vec![bumped.id(), other.id()];
    expected.sort_unstable();
    assert_eq!(held, expected);
    assert_eq!(page.actors().len(), 1, "a page reads only what is under it");

    let read = window.actor(bumped.id()).map(|actor| actor.state);
    assert_eq!(read, Some(format!("{:#?}", Ticker(1))));
    assert!(window.actor(usize::MAX).is_none());

    window.remove();
    assert!(window.actors().is_empty(), "what a removed scope held is not listed");
}

#[test]
fn a_reducer_is_read_where_it_was_claimed_or_from_the_nearest_export_above() {
    #[derive(Clone, Default, Debug)]
    struct Hidden;

    impl Reducer for Hidden {
        type Update = ();

        fn reduce(&mut self, _: ()) {}
    }

    let root = ScopeTree::new();
    let layout = root.child();
    let page = layout.child();
    root.note_reducer_owner::<Counter>();
    root.note_export::<Counter>();
    layout.note_reducer_owner::<Counter>();
    layout.note_export::<Counter>();
    layout.note_reducer_owner::<Hidden>();
    page.note_reducer_owner::<Hidden>();

    assert_eq!(page.owner_of::<Counter>(), Some(layout), "the nearest export wins");
    assert_eq!(page.owner_of::<Hidden>(), Some(page), "what it claimed is its own");
    assert_eq!(layout.child().owner_of::<Hidden>(), None, "claimed above is not exported");
    assert_eq!(root.owner_of::<Counter>(), Some(root.scope()));
}

#[test]
fn a_described_state_names_the_feature_that_claimed_it() {
    #[derive(Clone, Default, Debug)]
    struct Loose;

    impl Reducer for Loose {
        type Update = ();

        fn reduce(&mut self, _: ()) {}
    }

    let scope = ScopeTree::new();
    scope.open_section(Some("app::CounterFeature"), None);
    scope.note_reducer_owner::<Counter>();
    scope.state::<Counter>();
    scope.close_section();
    scope.state::<Loose>();

    let described = scope.describe_states();
    let feature = |name: &str| {
        described
            .iter()
            .find(|state| state.type_name.ends_with(name))
            .map(|state| state.feature)
    };
    assert_eq!(feature("Counter"), Some(Some("app::CounterFeature")));
    assert_eq!(feature("Loose"), Some(None));
}

#[test]
fn state_survives_unmount_remount_within_a_live_store() {
    let store = ScopeTree::new();

    let first_read = store.state::<Counter>();
    assert_eq!(first_read.borrow().value, 0);
    store.push::<Counter>(CounterMsg::Set(42));

    drop(first_read);

    let second_read = store.state::<Counter>();
    assert_eq!(second_read.borrow().value, 42);
}

#[test]
fn push_notifies_subscribers_and_unsubscribe_stops_it() {
    let store = ScopeTree::new();
    let seen = Rc::new(RefCell::new(Vec::new()));

    let seen_for_sub = seen.clone();
    let read = store.scope();
    let sub = store.subscribe::<Counter>(move || {
        seen_for_sub.borrow_mut().push(read.state::<Counter>().borrow().value);
    });

    store.push::<Counter>(CounterMsg::Set(1));
    store.push::<Counter>(CounterMsg::Set(2));
    assert_eq!(*seen.borrow(), vec![1, 2]);

    drop(sub);
    store.push::<Counter>(CounterMsg::Set(3));
    assert_eq!(
        *seen.borrow(),
        vec![1, 2],
        "no further notifications after the Subscription is dropped"
    );
}

#[tokio::test]
async fn removing_the_store_aborts_owned_tasks() {
    let ran_to_completion = Arc::new(AtomicBool::new(false));
    let flag = ran_to_completion.clone();

    let store = ScopeTree::new();
    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        flag.store(true, Ordering::SeqCst);
    });
    store.own(handle);

    store.remove();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !ran_to_completion.load(Ordering::SeqCst),
        "task should have been aborted when its owning scope was removed"
    );
}

#[test]
fn own_actor_disposes_the_registry_entry_on_removal() {
    let token = crate::actor::UiThreadToken::dangerously_create_token_unchecked();
    let addr = Addr::new_scoped((), token);
    let counter = addr.strong_count_ptr();

    let store = ScopeTree::new();
    store.own(addr.clone());
    drop(addr);

    assert!(
        Rc::strong_count(&counter) > 1,
        "REGISTRY should still hold the actor alive while its Scope is alive"
    );

    store.remove();

    assert_eq!(
        Rc::strong_count(&counter),
        1,
        "removing the Scope should dispose the REGISTRY entry, \
         leaving only this test's own counter handle"
    );
}
