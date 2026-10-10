//! The direction a chain comes apart in.
//!
//! Installing runs outermost first, because a page reads what the layout above
//! it installed and that has to exist before the page does. Teardown is the
//! same statement backwards: the reader goes first, and what it reads outlives
//! it.
//!
//! This used to run the other way. It never crashed - an update from a segment
//! being torn down landed nowhere - which is exactly why it needed a test
//! rather than a bug report.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext, Listed, Lists};
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{RouteChain, Router, SegmentEntry};

thread_local! {
    /// Who was torn down, in the order it happened.
    static GONE: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Something only one segment holds, so its drop dates that segment's.
struct Marks(&'static str);

impl Drop for Marks {
    fn drop(&mut self) {
        GONE.with(|gone| gone.borrow_mut().push(self.0));
    }
}

/// Neither a feature nor a reducer: nothing for a check of the tree to read.
impl Lists for Marks {
    fn list(_into: &mut Vec<Listed>) {}
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = Marks;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Marks> {
        Ok(Marks("Shell"))
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Tabs;

impl Layout for Tabs {
    type Params = ();
    type Installs = Marks;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Marks> {
        Ok(Marks("Tabs"))
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Processes;

impl Page for Processes {
    type Params = ();
    type Installs = Marks;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Marks> {
        Ok(Marks("Processes"))
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// A sibling of `Processes` under the same layouts, so navigating between them
/// keeps the whole prefix and takes down only the leaf.
struct Services;

impl Page for Services {
    type Params = ();
    type Installs = Marks;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Marks> {
        Ok(Marks("Services"))
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const PROCESSES: [SegmentEntry<Headless>; 3] = [
    layout_entry::<Shell>(),
    layout_entry::<Tabs>(),
    segment_entry::<Processes>(),
];

const SERVICES: [SegmentEntry<Headless>; 3] = [
    layout_entry::<Shell>(),
    layout_entry::<Tabs>(),
    segment_entry::<Services>(),
];

fn params() -> Vec<Box<dyn Any>> {
    vec![Box::new(()), Box::new(()), Box::new(())]
}

/// What `routes!` would generate, by hand: `activate` always installs from
/// scratch, and keeping a shared prefix is something only `navigate` does.
#[derive(Clone, PartialEq)]
enum Route {
    Processes,
    Services,
}

impl RouteChain<Headless> for Route {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        match self {
            Route::Processes => &PROCESSES,
            Route::Services => &SERVICES,
        }
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        params()
    }

    fn name(&self) -> &'static str {
        match self {
            Route::Processes => "Processes",
            Route::Services => "Services",
        }
    }
}

fn router() -> Rc<Router<Headless>> {
    GONE.with(|gone| gone.borrow_mut().clear());

    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    router.navigate(Route::Processes).expect("the first route");
    router
}

fn gone() -> Vec<&'static str> {
    GONE.with(|gone| gone.borrow().clone())
}

#[test]
fn a_chain_comes_apart_from_the_inside() {
    let router = router();
    router.deactivate();

    assert_eq!(
        gone(),
        ["Processes", "Tabs", "Shell"],
        "a segment reads what the ones above it installed, so it goes first"
    );
}

#[test]
fn dropping_the_router_takes_it_apart_the_same_way() {
    // The path a closing window takes, and the one where the order is hardest
    // to notice going wrong.
    let router = router();
    drop(router);

    assert_eq!(gone(), ["Processes", "Tabs", "Shell"]);
}

#[test]
fn only_what_leaves_is_torn_down() {
    let router = router();
    router.navigate(Route::Services).expect("to the sibling");

    assert_eq!(
        gone(),
        ["Processes"],
        "the shared prefix was not reinstalled, so it was not torn down either"
    );

    router.deactivate();
    assert_eq!(gone(), ["Processes", "Services", "Tabs", "Shell"]);
}

/// What a view keeps of the chain names the scopes and does not keep them: the
/// chain comes apart when the router takes it apart, in the same order,
/// whoever still has the list.
#[test]
fn a_chain_a_view_still_holds_comes_apart_all_the_same() {
    let router = router();
    let held = router.active_scopes().expect("a chain is mounted");

    router.deactivate();

    assert_eq!(gone(), ["Processes", "Tabs", "Shell"]);
    drop(held);
}

/// An immediate-mode backend draws *from* the chain, so a navigation asked for
/// inside the drawing must not take it apart yet: every teardown would run
/// under a frame that is still reading the scopes.
#[test]
fn a_navigation_from_inside_a_frame_waits_for_the_frame() {
    let router = router();

    {
        let _drawing = router.drawing();
        router.navigate(Route::Services).expect("held until the frame ends");

        assert!(gone().is_empty(), "nothing comes apart underneath the drawing");
    }

    assert!(
        gone().is_empty(),
        "the frame being over is not itself the navigation"
    );
    assert!(router.settle().expect("settling"), "it happens now");
    assert_eq!(gone(), ["Processes"]);

    assert!(
        !router.settle().expect("nothing left"),
        "and only once"
    );
}
