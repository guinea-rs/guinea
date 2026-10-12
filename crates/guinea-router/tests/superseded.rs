//! A navigation that has not happened yet - held for the end of a frame, or
//! waiting on a guard's question - and a later one that replaces it.
//!
//! Last intent wins: whatever was asked for later is where the router goes,
//! and what it replaced is settled as not having arrived.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::guard::{Ask, Decision, Verdict};
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{NavigateHandle, RouteChain, RouteSink, Router, SegmentEntry};

thread_local! {
    static ASKING: Cell<bool> = const { Cell::new(false) };
    static SHELLS_STANDING: Cell<i32> = const { Cell::new(0) };
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        SHELLS_STANDING.with(|n| n.set(n.get() + 1));
        ctx.scope.on_teardown(|| SHELLS_STANDING.with(|n| n.set(n.get() - 1)));
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Asks before it is left, while `ASKING` is set.
struct Editor;

impl Page for Editor {
    type Params = ();
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        ctx.on_leave(|| match ASKING.with(|asking| asking.get()) {
            false => Verdict::Allow,
            true => Verdict::Ask(Ask::new("Unsaved changes", "Discard", "Stay"), Decision::new()),
        });
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Third;

impl Page for Third {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Outside the shell.
struct Other;

impl Page for Other {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const EDITOR: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Editor>()];
const THIRD: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Third>()];
const OTHER: [SegmentEntry<Headless>; 1] = [segment_entry::<Other>()];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Route {
    Editor,
    Third,
    Other,
}

impl RouteChain<Headless> for Route {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        match self {
            Route::Editor => &EDITOR,
            Route::Third => &THIRD,
            Route::Other => &OTHER,
        }
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        self.chain().iter().map(|_| Box::new(()) as Box<dyn Any>).collect()
    }

    fn name(&self) -> &'static str {
        match self {
            Route::Editor => "Editor",
            Route::Third => "Third",
            Route::Other => "Other",
        }
    }
}

fn opened() -> Rc<Router<Headless>> {
    ASKING.with(|asking| asking.set(false));
    SHELLS_STANDING.with(|n| n.set(0));

    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    router.navigate(Route::Editor).expect("navigate");
    router
}

#[test]
fn a_navigation_after_the_frame_supersedes_what_the_frame_held() {
    let router = opened();

    {
        let _drawing = router.drawing();
        router.navigate(Route::Third).expect("held");
    }
    router.navigate(Route::Other).expect("navigate");
    router.settle().expect("settle");

    assert_eq!(router.current_route::<Route>(), Some(Route::Other), "last intent wins");
    assert_eq!(SHELLS_STANDING.with(|n| n.get()), 0, "Other stands outside the shell");
}

#[test]
fn a_question_asked_during_a_frame_supersedes_what_the_frame_held() {
    let router = opened();

    {
        let _drawing = router.drawing();
        router.navigate(Route::Other).expect("held");
        ASKING.with(|asking| asking.set(true));
        router.navigate(Route::Third).expect("asked");
    }
    router.settle().expect("settle");
    router.answer(true);

    assert_eq!(router.current_route::<Route>(), Some(Route::Third));
    assert_eq!(
        SHELLS_STANDING.with(|n| n.get()),
        1,
        "Third stands in a chain whose first segment is Shell"
    );
}

#[test]
fn what_a_frame_held_goes_in_against_the_chain_standing_when_it_does() {
    let router = opened();

    {
        let _drawing = router.drawing();
        router.navigate(Route::Third).expect("held");
    }
    router.activate(&OTHER, vec![Box::new(())]).expect("activate");
    router.settle().expect("settle");

    assert_eq!(router.current_route::<Route>(), Some(Route::Third));
    assert_eq!(
        SHELLS_STANDING.with(|n| n.get()),
        1,
        "the shell it was held beside is gone, so it goes up again"
    );
}

/// A router that went to each of `through` in turn.
fn travelled(through: &[Route]) -> (Rc<Router<Headless>>, NavigateHandle<Headless, Route>) {
    ASKING.with(|asking| asking.set(false));
    SHELLS_STANDING.with(|n| n.set(0));

    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    let nav = NavigateHandle::new(router.clone(), RouteSink::new(|_: Route| {}));

    for route in through {
        nav.to(*route);
    }

    (router, nav)
}

#[test]
fn a_back_that_supersedes_a_pending_back_is_one_step_from_where_the_router_stands() {
    let (router, nav) = travelled(&[Route::Other, Route::Third, Route::Editor]);

    ASKING.with(|asking| asking.set(true));
    nav.back();
    nav.back();
    router.answer(true);
    ASKING.with(|asking| asking.set(false));

    let landed = router.current_route::<Route>();
    nav.back();
    let behind = router.current_route::<Route>();

    assert_eq!(
        (landed, behind),
        (Some(Route::Third), Some(Route::Other)),
        "the first back never happened, so the second went one step from Editor"
    );
}

#[test]
fn a_forward_that_supersedes_a_pending_forward_is_one_step_from_where_the_router_stands() {
    let (router, nav) = travelled(&[Route::Editor, Route::Third, Route::Other]);
    nav.back();
    nav.back();

    ASKING.with(|asking| asking.set(true));
    nav.forward();
    nav.forward();
    router.answer(true);
    ASKING.with(|asking| asking.set(false));

    let landed = router.current_route::<Route>();
    nav.forward();
    let ahead = router.current_route::<Route>();

    assert_eq!((landed, ahead), (Some(Route::Third), Some(Route::Other)));
}

#[test]
fn a_held_navigation_that_was_superseded_never_arrives() {
    let router = opened();
    let arrived = Rc::new(Cell::new(None));

    {
        let _drawing = router.drawing();
        let seen = arrived.clone();
        router
            .navigate_then(Route::Third, move |done| seen.set(Some(done)))
            .expect("held");
    }
    router.navigate(Route::Other).expect("navigate");

    assert_eq!(arrived.get(), Some(false), "superseded, so it never arrived");
    assert!(!router.settle().expect("settle"), "nothing is left to settle");
}
