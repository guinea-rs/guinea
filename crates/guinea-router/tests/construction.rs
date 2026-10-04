//! A route tree, checked as a whole before anything in it mounts.
//!
//! Reach is resolved where a read is made, so what used to be refused by the
//! compiler is now refused here: the tree is read from what each segment
//! says it `Installs`, and what each feature says it `Exports`, before the
//! first navigation installs anything.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use guinea_app::feature::{Feature, FeatureHost, FeatureInitContext};
use guinea_core::actor::UiThreadToken;
use guinea_core::feature::Bound;
use guinea_core::scope::Reducer;
use guinea_macros::routes;
use guinea_router::construction::check;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{Router, SegmentEntry};

#[derive(Default, Clone, Debug)]
struct Shown(u32);

impl Reducer for Shown {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

thread_local! {
    static INSTALLED: Cell<usize> = const { Cell::new(0) };
}

struct Chrome;

impl Feature for Chrome {
    type Params = ();
    type Exports = (Shown,);

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        INSTALLED.set(INSTALLED.get() + 1);
        cx.state::<Shown>().plain();
        Ok(Self)
    }
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

/// Installs again what its layout already did.
struct Doubling;

impl Page for Doubling {
    type Params = ();
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Claims for itself what its layout exports.
struct Shadowing;

impl Page for Shadowing {
    type Params = ();
    type Installs = Bound<Shown>;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Bound<Shown>> {
        Ok(ctx.state::<Shown>().plain())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Installs the feature, with no layout above that did.
struct Alone;

impl Page for Alone {
    type Params = ();
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Reader;

impl Page for Reader {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// A layout under `Shell` that installs what `Shell` did.
struct Inner;

impl Layout for Inner {
    type Params = ();
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

/// Lists one feature twice.
struct Greedy;

impl Page for Greedy {
    type Params = ();
    type Installs = (Chrome, Chrome);

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Chrome, Chrome)> {
        Ok((ctx.install::<Chrome>(&())?, ctx.install::<Chrome>(&())?))
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const DOUBLED: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Doubling>()];
const TWICE_ABOVE_READ: [SegmentEntry<Headless>; 3] = [
    layout_entry::<Shell>(),
    layout_entry::<Inner>(),
    segment_entry::<Reader>(),
];
const TWICE_ABOVE_SHADOWING: [SegmentEntry<Headless>; 3] = [
    layout_entry::<Shell>(),
    layout_entry::<Inner>(),
    segment_entry::<Shadowing>(),
];
const LISTED_TWICE: [SegmentEntry<Headless>; 1] = [segment_entry::<Greedy>()];
const SHADOWED: [SegmentEntry<Headless>; 2] =
    [layout_entry::<Shell>(), segment_entry::<Shadowing>()];
const READ: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Reader>()];
const ALONE: [SegmentEntry<Headless>; 1] = [segment_entry::<Alone>()];

#[test]
fn a_feature_installed_twice_on_one_chain_is_refused() {
    let construction = check(&[&DOUBLED]);

    assert_eq!(
        construction.errors,
        ["`Chrome` is installed by `Shell`, and again by `Doubling` below it"]
    );
}

#[test]
fn a_feature_in_two_branches_is_installed_once_in_each() {
    let construction = check(&[&READ, &ALONE]);

    assert!(construction.errors.is_empty(), "{:?}", construction.errors);
    assert!(construction.warnings.is_empty(), "{:?}", construction.warnings);
}

#[test]
fn a_prefix_shared_by_two_routes_is_reported_once() {
    let construction = check(&[&TWICE_ABOVE_READ, &TWICE_ABOVE_SHADOWING]);

    assert_eq!(
        construction.errors,
        ["`Chrome` is installed by `Shell`, and again by `Inner` below it"]
    );
}

#[test]
fn a_feature_listed_twice_by_one_segment_is_refused() {
    let construction = check(&[&LISTED_TWICE]);

    assert_eq!(
        construction.errors,
        ["`Chrome` is listed twice in what `Greedy` installs"]
    );
}

#[test]
fn a_reducer_exported_twice_on_one_chain_is_a_warning() {
    let construction = check(&[&SHADOWED]);

    assert!(construction.errors.is_empty(), "{:?}", construction.errors);
    assert_eq!(
        construction.warnings,
        [
            "`Shown` is exported by `Shell`, and again by `Shadowing` below it: \
             `Shadowing` and everything below it read the one `Shadowing` installs"
        ]
    );
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Shell) {
            page(Doubling)
            page(Reader)
        }
    }
}

#[test]
fn the_first_navigation_refuses_a_tree_built_wrong_before_installing_anything() {
    INSTALLED.set(0);
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached(token)));

    let outcome = catch_unwind(AssertUnwindSafe(|| router.navigate(Route::Reader {}).map(|_| ())));

    let message = match &outcome {
        Err(panic) => panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string())),
        Ok(_) => None,
    };
    assert!(
        message
            .as_deref()
            .is_some_and(|it| it.contains("`Chrome` is installed by `Shell`, and again by `Doubling`")),
        "the route taken was fine, the tree it belongs to was not; got {message:?}"
    );
    assert_eq!(INSTALLED.get(), 0, "nothing installed before the tree was checked");
}
