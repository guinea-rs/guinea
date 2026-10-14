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
use guinea_macros::reducer;
use guinea_router::headless::{Headless, HeadlessCx, Page, segment_entry};
use guinea_router::router::{Router, SegmentEntry};

thread_local! {
    /// Set when a page's scope is torn down.
    static TORN_DOWN: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Default, Debug)]
struct Marker(u32);

#[reducer]
fn marker(this: &mut Marker, to: u32) {
    this.0 = to;
}

struct Processes;

impl Page for Processes {
    type Params = ();
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        ctx.state::<Marker>().seed(Marker(1)).plain();
        ctx.scope.on_teardown(|| TORN_DOWN.with(|torn| torn.set(true)));
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
    let window = router.host().scope();

    assert_eq!(roots::of(window), Some(id));

    drop(router);

    assert_eq!(roots::of(window), None, "the root goes with the host");
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
    let (gone, staying) = (first.host().scope(), second.host().scope());

    assert_ne!(first.root(), second.root(), "each host is its own root");

    drop(first);

    assert_eq!(roots::of(gone), None);
    assert_eq!(roots::of(staying), Some(second.root()));
    assert!(
        second.active_scope().is_some(),
        "the surviving root still has its chain installed"
    );
}
