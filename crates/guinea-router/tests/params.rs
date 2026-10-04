//! What the router asks about a capture, and what it does with the answer.
//!
//! The question is one: is this segment still holding the same thing? Two
//! decisions hang off it - which segments survive a navigation, and which
//! cached state is allowed back - and both used to be answered by proxy. The
//! chain's shape stood in for the first and the segment's type for the second,
//! and each proxy was wrong in a way that only showed up once a parameter
//! actually varied.

use std::any::Any;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::{Reducer, Scope};
use guinea_router::headless::{
    Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry,
};
use guinea_router::router::{RouteChain, Router, SegmentEntry};

/// Whatever a page and its layouts were reached with.
#[derive(Clone, Debug, PartialEq)]
struct Context {
    host: String,
}

/// State nothing derives from the route - so if it comes back, it came back
/// from the cache rather than from a fresh install.
#[derive(Clone, Default, Debug)]
struct Scratch(u32);

impl Reducer for Scratch {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

/// Two layouts of the same shape, so a page can sit at the same depth under
/// either.
struct TabsA;
struct TabsB;

impl Layout for TabsA {
    type Params = Context;
    type Installs = ();

    fn install(_cx: &FeatureInitContext, _params: &Context) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

impl Layout for TabsB {
    type Params = Context;
    type Installs = ();

    fn install(_cx: &FeatureInitContext, _params: &Context) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Detail;

impl Page for Detail {
    const CACHE_STATE_IN_MEMORY: bool = true;

    type Params = Context;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &Self::Params) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Sibling;

impl Page for Sibling {
    type Params = Context;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &Context) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const UNDER_A: [SegmentEntry<Headless>; 2] = [layout_entry::<TabsA>(), segment_entry::<Detail>()];
const UNDER_B: [SegmentEntry<Headless>; 2] = [layout_entry::<TabsB>(), segment_entry::<Detail>()];
const SIBLING: [SegmentEntry<Headless>; 2] = [layout_entry::<TabsA>(), segment_entry::<Sibling>()];

#[derive(Clone)]
enum Route {
    UnderA(String),
    UnderB(String),
    Sibling(String),
}

impl RouteChain<Headless> for Route {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        match self {
            Route::UnderA(_) => &UNDER_A,
            Route::UnderB(_) => &UNDER_B,
            Route::Sibling(_) => &SIBLING,
        }
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        let (Route::UnderA(host) | Route::UnderB(host) | Route::Sibling(host)) = self;
        let context = Context { host: host.clone() };
        // Both segments carry it: the layout's is what `routes!` derives as
        // the intersection of its pages'.
        vec![Box::new(context.clone()), Box::new(context)]
    }

    fn name(&self) -> &'static str {
        match self {
            Route::UnderA(_) => "UnderA",
            Route::UnderB(_) => "UnderB",
            Route::Sibling(_) => "Sibling",
        }
    }
}

fn router() -> Rc<Router<Headless>> {
    let token = UiThreadToken::dangerously_create_token_unchecked();
    Rc::new(Router::<Headless>::new(FeatureHost::detached(token)))
}

fn go(router: &Rc<Router<Headless>>, route: Route) {
    router.navigate(route).expect("navigate");
}

fn scratch(scope: Scope) -> u32 {
    scope.state::<Scratch>().borrow().0
}

#[test]
fn a_layout_stays_while_what_it_derives_stays() {
    let router = router();

    go(&router, Route::UnderA("ubuntu".into()));
    let first = router.scope_at(0).expect("a layout is mounted");

    go(&router, Route::Sibling("ubuntu".into()));
    let second = router.scope_at(0).expect("a layout is mounted");

    assert_eq!(
        first, second,
        "only the page under it changed, and the layout captured the same host"
    );
}

#[test]
fn a_layout_reinstalls_when_what_it_derives_changes() {
    let router = router();

    go(&router, Route::UnderA("ubuntu".into()));
    let first = router.scope_at(0).expect("a layout is mounted");

    go(&router, Route::UnderA("fedora".into()));
    let second = router.scope_at(0).expect("a layout is mounted");

    assert_ne!(
        first, second,
        "the chain is the same shape, but the layout was handed a different \
         host - keeping it would leave a layout built for one showing another"
    );
}

#[test]
fn a_cached_page_comes_back_to_the_capture_it_left() {
    let router = router();

    go(&router, Route::UnderA("ubuntu".into()));
    router
        .scope_at(1)
        .expect("a page is mounted")
        .push::<Scratch>(7);

    go(&router, Route::Sibling("ubuntu".into()));
    go(&router, Route::UnderA("ubuntu".into()));

    assert_eq!(
        scratch(router.scope_at(1).expect("a page is mounted")),
        7,
        "same host, so the state it was cached with is still its own"
    );
}

#[test]
fn a_cached_page_does_not_come_back_to_a_different_capture() {
    let router = router();

    go(&router, Route::UnderA("ubuntu".into()));
    router
        .scope_at(1)
        .expect("a page is mounted")
        .push::<Scratch>(7);

    go(&router, Route::Sibling("ubuntu".into()));
    go(&router, Route::UnderA("fedora".into()));

    assert_eq!(
        scratch(router.scope_at(1).expect("a page is mounted")),
        0,
        "a page cached under one host is a different page's worth of state \
         under another"
    );
}

#[test]
fn the_same_page_under_two_layouts_is_two_places() {
    let router = router();

    go(&router, Route::UnderA("ubuntu".into()));
    router
        .scope_at(1)
        .expect("a page is mounted")
        .push::<Scratch>(7);

    go(&router, Route::UnderB("ubuntu".into()));

    assert_eq!(
        scratch(router.scope_at(1).expect("a page is mounted")),
        0,
        "same type at the same depth, but under a different layout - keying \
         the cache on depth and type alone would have let one restore into \
         the other"
    );
}
