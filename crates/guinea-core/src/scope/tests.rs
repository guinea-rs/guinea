use super::*;
use std::cell::RefCell;

use crate::actor::Addr;

#[derive(Default, Clone, PartialEq, Debug)]
struct Counter {
    value: i32,
}

#[derive(Clone)]
enum CounterMsg {
    Set(i32),
}

#[guinea_macros::reducer]
fn counter(this: &mut Counter, update: CounterMsg) {
    match update {
        CounterMsg::Set(v) => this.value = v,
    }
}

type Said = Rc<RefCell<Vec<&'static str>>>;

fn says(said: &Said, name: &'static str) -> impl FnOnce() + 'static {
    let said = said.clone();
    move || said.borrow_mut().push(name)
}

#[test]
fn a_tree_takes_every_scope_in_it_along_when_its_owner_lets_go() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let tree = ScopeTree::new();
    let window = tree.child();
    let page = window.child();
    tree.on_teardown(says(&said, "application"));
    page.on_teardown(says(&said, "page"));

    drop(tree);

    assert_eq!(*said.borrow(), ["page", "application"]);
    assert!(!window.is_alive());
    assert!(!page.is_alive());
}

#[test]
fn a_wake_hook_does_not_run_after_an_earlier_hook_removed_its_scope() {
    let tree = ScopeTree::new();
    let page = tree.child();
    let late = Rc::new(std::cell::Cell::new(0));

    page.on_wake(move || page.remove());
    let counted = late.clone();
    page.on_wake(move || counted.set(counted.get() + 1));
    page.sleep();

    page.wake();

    assert!(!page.is_alive());
    assert_eq!(late.get(), 0, "a removed page's wake hook ran");
}

#[test]
fn a_wake_hook_does_not_run_after_an_earlier_hook_put_its_scope_back_to_sleep() {
    let tree = ScopeTree::new();
    let page = tree.child();
    let late = Rc::new(std::cell::Cell::new(0));

    page.on_wake(move || page.sleep());
    let counted = late.clone();
    page.on_wake(move || counted.set(counted.get() + 1));
    page.sleep();

    page.wake();

    assert!(!page.is_awake());
    assert_eq!(late.get(), 0, "it catches up at the next wake, not while asleep");
}

#[test]
fn a_reducer_only_read_is_described() {
    let tree = ScopeTree::new();
    tree.drawn_by(crate::draws::Calls);
    let _reading = tree.binding::<Counter>().read_by(Box::new(|| {}) as Box<dyn Fn()>);

    let described: Vec<_> = tree.describe_states().iter().map(|state| state.type_name).collect();

    assert_eq!(described, [std::any::type_name::<Counter>()], "the scope holds a Counter cell");
}

#[test]
fn a_reducer_only_observed_is_described() {
    let tree = ScopeTree::new();
    let _observing = tree.observe::<Counter>(|_| {});

    let described: Vec<_> = tree.describe_states().iter().map(|state| state.type_name).collect();

    assert_eq!(described, [std::any::type_name::<Counter>()], "the scope holds a Counter cell");
}

#[derive(Clone)]
struct Ping;

impl crate::actor::event_bus::Event for Ping {}

#[test]
fn two_trees_on_one_thread_hear_only_what_is_published_on_their_own_bus() {
    let ours = ScopeTree::new();
    let theirs = ScopeTree::new();
    let heard = Rc::new(std::cell::Cell::new(0));

    let hearing = heard.clone();
    let _subscription = ours.child().bus().subscribe_fn(move |_: Ping| hearing.set(hearing.get() + 1));

    theirs.bus().publish(Ping);
    ours.bus().publish(Ping);

    assert_eq!(heard.get(), 1);
}

#[test]
fn every_scope_in_a_tree_has_the_tree_s_bus() {
    let tree = ScopeTree::new();
    let page = tree.child().child();

    assert!(Rc::ptr_eq(&tree.bus(), &page.bus()));
}

#[test]
fn a_scope_of_a_tree_that_is_gone_reaches_no_tree_that_came_after() {
    let gone = ScopeTree::new();
    let stale = gone.child();
    drop(gone);
    let next = ScopeTree::new();

    assert!(
        stale.tree_bus().is_none(),
        "the tree after it took its place, and the stale scope heard its bus"
    );
    assert!(next.is_alive());
}

#[test]
fn a_scope_of_a_tree_that_is_gone_is_not_one_of_the_tree_that_took_its_place() {
    let gone = ScopeTree::new();
    let stale = gone.scope();
    drop(gone);
    let next = ScopeTree::new();

    assert_ne!(stale, next.scope());
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
        scope.on_teardown(says(&said, name));
    }
    root.remove();

    assert_eq!(*said.borrow(), ["newer", "below", "older", "root"]);
}

#[test]
fn a_scope_lets_go_of_what_it_holds_in_the_reverse_of_the_order_it_took_it_whatever_it_is() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let scope = ScopeTree::new();
    scope.on_teardown(says(&said, "store"));
    drop(Addr::new(Leaving::saying(&said, "actor"), &home(scope.scope())));
    scope.on_teardown(says(&said, "reading the store"));

    scope.remove();

    assert_eq!(*said.borrow(), ["reading the store", "actor", "store"]);
}

struct Says(Said, &'static str);

impl Drop for Says {
    fn drop(&mut self) {
        self.0.borrow_mut().push(self.1);
    }
}

/// An actor that says so when it goes.
struct Leaving {
    _says: Says,
}

impl Leaving {
    fn saying(said: &Said, name: &'static str) -> Self {
        Self {
            _says: Says(said.clone(), name),
        }
    }
}

impl std::fmt::Debug for Leaving {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Leaving")
    }
}

guinea_macros::actor! {
    Leaving {
        handlers {
            Tick
        }
    }
}

impl crate::actor::Handler<Tick> for Leaving {
    fn handle(_: crate::actor::In<Self, Tick>) {}
}

#[tokio::test]
async fn a_scope_lets_go_of_every_kind_it_holds_in_the_reverse_of_the_order_it_took_them() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let scope = ScopeTree::new();
    scope.drawn_by(crate::draws::Calls);

    let hearing = Says(said.clone(), "hearing");
    scope.hold_hearing(scope.bus().subscribe_fn(move |_: Ping| {
        let _ = &hearing;
    }));
    let watching = Says(said.clone(), "watching");
    let reader: Box<dyn Fn()> = Box::new(move || {
        let _ = &watching;
    });
    scope.hold_watching(scope.binding::<Counter>().read_by(reader));
    let timer = Rc::new(RefCell::new(None::<crate::sources::Source>));
    let after_the_timer = said.clone();
    let gone = timer.clone();
    scope.on_teardown(move || {
        let closed = gone.borrow().as_ref().is_some_and(|timer| timer.id().is_none());
        after_the_timer
            .borrow_mut()
            .push(if closed { "timer" } else { "timer still open" });
    });
    timer.replace(Some(scope.section().repeat(std::time::Duration::from_secs(60), || {})));
    drop(Addr::new(Leaving::saying(&said, "actor"), &home(scope.scope())));
    scope.on_teardown(says(&said, "cleanup"));

    scope.remove();

    assert_eq!(
        *said.borrow(),
        ["cleanup", "actor", "timer", "watching", "hearing"]
    );
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
    assert_eq!(next.parent(), Some(root.scope()));
}

#[test]
fn a_scope_goes_when_it_is_removed_however_many_name_it() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let tree = ScopeTree::new();
    let scope = tree.child();
    let named_elsewhere = scope;
    scope.on_teardown(says(&said, "torn down"));

    scope.remove();

    assert_eq!(*said.borrow(), ["torn down"]);
    assert!(!named_elsewhere.is_alive());
}

#[test]
fn what_a_removed_scope_is_handed_is_torn_down_at_once() {
    let said = Rc::new(RefCell::new(Vec::new()));
    let scope = ScopeTree::new();
    scope.remove();

    scope.on_teardown(says(&said, "at once"));

    assert_eq!(*said.borrow(), ["at once"]);
}

fn refused_in_a_removed_scope(doing: &str, act: impl FnOnce(Scope)) {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    tree.remove();

    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| act(scope)));

    let message = done
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|message| *message);
    assert_eq!(message, Some(format!("{doing} in {scope:?}, which was removed")));
}

#[test]
fn pushing_into_a_removed_scope_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("pushing an update", |scope| {
        scope.push::<Counter>(CounterMsg::Set(1))
    });
}

#[test]
fn reading_in_a_removed_scope_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("reading a reducer", |scope| {
        drop(scope.binding::<Counter>().read_by(()))
    });
}

#[test]
fn observing_in_a_removed_scope_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("observing a reducer", |scope| {
        drop(scope.observe::<Counter>(|_| {}))
    });
}

#[test]
fn reading_a_reducer_of_a_removed_scope_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("reading a reducer", |scope| {
        drop(scope.state::<Counter>())
    });
}

#[test]
fn putting_a_removed_scope_to_sleep_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("putting to sleep", |scope| scope.sleep());
}

#[test]
fn waking_a_removed_scope_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("waking", |scope| scope.wake());
}

#[test]
fn noting_a_listener_in_a_removed_scope_is_a_mistake_said_out_loud() {
    refused_in_a_removed_scope("noting a listener", |scope| {
        scope.section().note_listener("Ping", None)
    });
}

fn refused_after_its_scope_went<T>(doing: &str, make: impl FnOnce(Scope) -> T, act: impl FnOnce(T)) {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    let made = make(scope);
    tree.remove();

    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| act(made)));

    let message = done
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|message| *message);
    assert_eq!(message, Some(format!("{doing} in {scope:?}, which was removed")));
}

#[test]
fn pushing_through_a_push_after_its_scope_went_is_a_mistake_said_out_loud() {
    refused_after_its_scope_went(
        "pushing an update",
        crate::feature::Push::<Counter>::new,
        |push| push.send(CounterMsg::Set(1)),
    );
}

#[test]
fn pushing_through_a_binding_after_its_scope_went_is_a_mistake_said_out_loud() {
    refused_after_its_scope_went(
        "pushing an update",
        |scope| scope.binding::<Counter>(),
        |binding| binding.push(CounterMsg::Set(1)),
    );
}

#[test]
fn watching_through_a_binding_after_its_scope_went_is_a_mistake_said_out_loud() {
    refused_after_its_scope_went(
        "reading a reducer",
        |scope| scope.binding::<Counter>(),
        |binding| drop(binding.read_by(())),
    );
}

#[test]
fn emitting_to_a_scope_that_went_is_a_mistake_said_out_loud() {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    tree.section().answers::<Tick>(|_| {});
    let dispatch = crate::feature::Dispatch::owning::<Counter>(scope);
    tree.remove();

    let emitted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch.emit(Tick)));

    let message = emitted
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|message| *message);
    let tick = crate::short_type_name::<Tick>();
    assert_eq!(message, Some(format!("emitting {tick} in {scope:?}, which was removed")));
}

#[test]
fn a_dispatcher_to_nowhere_drops_what_is_emitted_to_it() {
    let emitted = std::panic::catch_unwind(|| crate::feature::Dispatch::nowhere().emit(Tick));

    assert!(emitted.is_ok(), "a view drawn without a domain has nobody to ask");
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

fn home(scope: Scope) -> crate::actor::addr::Home {
    crate::actor::addr::Home::new(scope)
}

#[test]
fn a_window_reads_the_actors_under_it_and_one_by_id_without_the_rest() {
    let window = ScopeTree::new();
    let page = window.child();
    let other = Addr::new(Ticker(0), &home(window.scope()));
    let bumped = Addr::new(Ticker(0), &home(page));
    bumped.send(Tick);

    let mut held: Vec<usize> = window.actors().iter().map(|actor| actor.id).collect();
    held.sort_unstable();
    let mut expected = vec![bumped.id(), other.id()];
    expected.sort_unstable();
    assert_eq!(held, expected);
    assert_eq!(page.actors().len(), 1, "a page reads only what is under it");

    let read = window.actor(bumped.id()).map(|actor| actor.state.preview);
    assert_eq!(read.as_deref(), Some("Ticker(1)"));
    assert!(window.actor(usize::MAX).is_none());

    window.remove();
    assert!(window.actors().is_empty(), "what a removed scope held is not listed");
}

#[test]
fn a_reducer_is_read_where_it_was_claimed_or_from_the_nearest_export_above() {
    #[derive(Clone, Default, Debug)]
    struct Hidden;

    #[guinea_macros::reducer]
    fn hidden(_this: &mut Hidden, _: ()) {}

    let root = ScopeTree::new();
    let layout = root.child();
    let page = layout.child();
    root.section().claim::<Counter>(None);
    root.note_export::<Counter>();
    layout.section().claim::<Counter>(None);
    layout.note_export::<Counter>();
    layout.section().claim::<Hidden>(None);
    page.section().claim::<Hidden>(None);

    assert_eq!(page.owner_of::<Counter>(), Some(layout), "the nearest export wins");
    assert_eq!(page.owner_of::<Hidden>(), Some(page), "what it claimed is its own");
    assert_eq!(layout.child().owner_of::<Hidden>(), None, "claimed above is not exported");
    assert_eq!(root.owner_of::<Counter>(), Some(root.scope()));
}

#[test]
fn what_a_scope_keeps_is_handed_back_as_it_was_left() {
    let scope = ScopeTree::new();
    scope.keep(1u32);

    scope.kept::<u32, _>(|kept| *kept += 1);

    assert_eq!(scope.kept::<u32, _>(|kept| *kept), Some(2));
}

#[test]
fn asking_a_scope_that_is_there_for_what_it_never_kept_is_a_mistake_said_out_loud() {
    let tree = ScopeTree::new();
    let scope = tree.scope();

    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.kept::<u32, _>(|kept| *kept += 5)
    }));

    let message = asked
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|message| *message);
    assert_eq!(message, Some(format!("{scope:?} keeps no u32")));
}

#[test]
fn trying_a_scope_for_what_it_never_kept_hands_back_nothing() {
    let tree = ScopeTree::new();
    let scope = tree.scope();

    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.try_kept::<u32, _>(|kept| *kept += 5)
    }));

    assert!(matches!(asked, Ok(None)), "what may not be kept is not a mistake to ask for");
}

#[test]
fn trying_for_what_is_out_for_a_call_already_is_a_mistake_said_out_loud() {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    scope.keep(1u32);

    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.try_kept::<u32, _>(|_| scope.try_kept::<u32, _>(|kept| *kept))
    }));

    let message = asked
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|message| *message);
    assert_eq!(message, Some(format!("{scope:?} has its u32 out for a call already")));
}

#[test]
fn asking_for_what_is_out_for_a_call_already_is_a_mistake_said_out_loud() {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    scope.keep(1u32);

    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.kept::<u32, _>(|_| scope.kept::<u32, _>(|kept| *kept))
    }));

    let message = asked
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|message| *message);
    assert_eq!(message, Some(format!("{scope:?} keeps no u32")));
}

#[test]
fn a_scope_that_went_keeps_nothing() {
    let tree = ScopeTree::new();
    let page = tree.child();
    page.keep(1u32);
    page.remove();

    assert_eq!(page.kept::<u32, _>(|kept| *kept), None);
}

#[test]
fn whether_a_scope_keeps_a_type_is_asked_without_taking_it() {
    let tree = ScopeTree::new();
    let page = tree.child();
    page.keep(1u32);

    let asked = (page.keeps::<u32>(), page.keeps::<i64>(), tree.scope().keeps::<u32>());
    page.remove();

    assert_eq!(asked, (true, false, false));
    assert!(!page.keeps::<u32>(), "a scope that went keeps nothing");
}

#[derive(Default)]
struct Counted(Option<Rc<std::cell::Cell<u32>>>);

impl Drop for Counted {
    fn drop(&mut self) {
        if let Some(dropped) = &self.0 {
            dropped.set(dropped.get() + 1);
        }
    }
}

#[test]
fn what_a_scope_keeps_goes_with_it() {
    let dropped = Rc::new(std::cell::Cell::new(0));
    let tree = ScopeTree::new();
    let page = tree.child();
    page.keep(Counted(Some(dropped.clone())));

    let while_there = dropped.get();
    page.remove();

    assert_eq!((while_there, dropped.get()), (0, 1));
}

#[test]
fn what_a_scope_removed_during_the_call_kept_goes_after_it() {
    let dropped = Rc::new(std::cell::Cell::new(0));
    let tree = ScopeTree::new();
    let page = tree.child();
    page.keep(Counted(Some(dropped.clone())));

    let during = page.kept::<Counted, _>(|_| {
        page.remove();
        dropped.get()
    });

    assert_eq!((during, dropped.get()), (Some(0), 1));
    assert_eq!(page.kept::<Counted, _>(|kept| kept.0.is_some()), None);
}

#[test]
fn a_claimed_reducer_is_not_a_feature_installed_there() {
    let scope = ScopeTree::new();
    scope.section().claim::<Counter>(None);

    assert!(scope.claims::<Counter>());
    assert!(!scope.has_feature::<Counter>());
}

#[test]
fn a_described_state_names_the_feature_that_claimed_it() {
    #[derive(Clone, Default, Debug)]
    struct Loose;

    #[guinea_macros::reducer]
    fn loose(_this: &mut Loose, _: ()) {}

    let scope = ScopeTree::new();
    let counting = scope.open_section(Some("app::CounterFeature"), None);
    counting.claim::<Counter>(None);
    scope.state::<Counter>();
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
fn a_push_redraws_its_readers_until_they_are_let_go_of() {
    let store = ScopeTree::new();
    store.drawn_by(crate::draws::Calls);
    let seen = Rc::new(RefCell::new(Vec::new()));

    let seen_for_sub = seen.clone();
    let read = store.scope();
    let reader: Box<dyn Fn()> = Box::new(move || {
        seen_for_sub.borrow_mut().push(read.state::<Counter>().borrow().value);
    });
    let sub = store.binding::<Counter>().read_by(reader);

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

mod services {
    use std::any::TypeId;
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;

    /// A service that says when it goes - once, whatever clones were made.
    #[derive(Clone)]
    struct Store {
        _dropped: Rc<Dropped>,
    }

    struct Dropped(Said);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.borrow_mut().push("store");
        }
    }

    #[test]
    fn a_dependency_is_visible_only_to_its_scope_and_descendants() {
        let tree = ScopeTree::new();
        let page = tree.child().child();

        page.provide(7u32, "test").unwrap();

        assert_eq!(tree.service::<u32>(), None);
        assert_eq!(page.service::<u32>(), Some(7));
        assert_eq!(page.child().service::<u32>(), Some(7));
        assert_eq!(tree.child().service::<u32>(), None);
        assert!(page.provides(TypeId::of::<u32>()));
        assert!(!page.provides(TypeId::of::<u64>()));
    }

    #[test]
    fn a_service_stays_on_the_ui_thread_when_it_has_to() {
        let tree = ScopeTree::new();
        let counter = Rc::new(Cell::new(0));

        tree.provide(counter.clone(), "test").unwrap();

        let taken = tree.service::<Rc<Cell<u32>>>().expect("provided");
        assert!(Rc::ptr_eq(&taken, &counter));
    }

    #[test]
    fn a_second_service_of_a_type_is_refused_with_who_provided_the_first() {
        let tree = ScopeTree::new();
        tree.provide(7u32, "first").unwrap();

        assert_eq!(tree.child().provide(9u32, "second"), Err("first"));
        assert_eq!(tree.service::<u32>(), Some(7));
    }

    #[test]
    fn a_service_goes_with_its_tree_after_everything_the_tree_held() {
        let said: Said = Rc::new(RefCell::new(Vec::new()));
        let tree = ScopeTree::new();
        let store = Store {
            _dropped: Rc::new(Dropped(said.clone())),
        };
        tree.provide(store, "test").unwrap();
        tree.on_teardown(says(&said, "cleanup"));

        let taken = tree.child().service::<Store>();
        drop(taken);
        assert_eq!(*said.borrow(), [] as [&str; 0]);

        drop(tree);
        assert_eq!(*said.borrow(), ["cleanup", "store"]);
    }

    #[test]
    fn a_local_dependency_is_released_when_its_scope_is_removed() {
        let said: Said = Rc::new(RefCell::new(Vec::new()));
        let tree = ScopeTree::new();
        let page = tree.child();
        page.provide(Store { _dropped: Rc::new(Dropped(said.clone())) }, "test").unwrap();
        page.on_teardown(says(&said, "cleanup"));
        page.remove();
        assert_eq!(*said.borrow(), ["cleanup", "store"]);
        assert!(tree.is_alive());
    }

    #[test]
    fn a_service_read_through_a_removed_scope_panics() {
        let tree = ScopeTree::new();
        tree.provide(7u32, "test").unwrap();
        let page = tree.child();
        page.remove();

        let read = catch_unwind(AssertUnwindSafe(|| page.service::<u32>()));

        assert!(read.is_err(), "read {read:?}");
    }
}
