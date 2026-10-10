//! What closing a root takes with it.
//!
//! The question a second window asks: when one goes away, does its half of
//! the application go with it, and does the other half survive? Answered on
//! the headless backend, where a root is a router and nothing else - no
//! window, no toolkit, no shell deciding when to exit.

use std::cell::Cell;
use std::rc::Rc;

use guinea_app::app::roots;
use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::scope::{DropGuard, Reducer};
use guinea_router::headless::{Headless, HeadlessCx, Page, segment_entry};
use guinea_router::router::{Router, SegmentEntry};

thread_local! {
    /// Set when a page's scope is torn down.
    static TORN_DOWN: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Default, Debug)]
struct Marker(u32);

impl Reducer for Marker {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

/// Notes its own teardown, the way an actor's `dispose` would.
struct Tombstone;

impl Drop for Tombstone {
    fn drop(&mut self) {
        TORN_DOWN.with(|torn| torn.set(true));
    }
}

struct Processes;

impl Page for Processes {
    type Params = ();
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        ctx.state::<Marker>().seed(Marker(1)).plain();
        ctx.scope.own(DropGuard(Tombstone));
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const CHAIN: [SegmentEntry<Headless>; 1] = [segment_entry::<Processes>()];

fn open() -> Rc<Router<Headless>> {
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    router
        .activate(&CHAIN, vec![Box::new(())])
        .expect("activate");
    router
}

#[test]
fn a_router_is_a_root_while_it_lives() {
    TORN_DOWN.with(|torn| torn.set(false));

    let router = open();
    let id = router.root();

    assert!(roots::is_open(id));
    assert!(roots().contains(&id));

    drop(router);

    assert!(!roots::is_open(id), "the entry goes with the host");
    assert!(
        TORN_DOWN.with(Cell::get),
        "and so do the scopes under it - closing a window has to end its features"
    );
}

#[test]
fn closing_one_root_leaves_the_other_running() {
    // The case a second window is opened for: two roots, sharing the process
    // and nothing else.
    let first = open();
    let second = open();
    let (gone, staying) = (first.root(), second.root());

    assert_ne!(gone, staying, "each host is its own root");

    drop(first);

    assert!(!roots::is_open(gone));
    assert!(roots::is_open(staying));
    assert!(
        second.active_scope().is_some(),
        "the surviving root still has its chain installed"
    );
}

fn roots() -> Vec<roots::RootId> {
    roots::roots()
}
