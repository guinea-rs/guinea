//! Leaving as a stage of navigation.
//!
//! The whole point is that deciding mutates nothing: a guard runs before any
//! scope is dropped and before any is built, so a refusal leaves the
//! application exactly where it was and a second navigation can replace the
//! first without cleaning anything up.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::actor::UiThreadToken;
use guinea_core::guard::{Ask, Decision, Verdict};
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{Navigation, RouteChain, Router, SegmentEntry};

thread_local! {
    /// What the guarded page answers next time it is asked. A test sets it
    /// before navigating away.
    static ANSWER: Cell<Reply> = const { Cell::new(Reply::Allow) };
    /// The token of the last question, so a test can answer it the way a
    /// dialog would.
    static ASKED: std::cell::RefCell<Option<Decision>> =
        const { std::cell::RefCell::new(None) };
    /// How many times the shell was built - what proves nothing was torn down
    /// while a question was open.
    static SHELL_INSTALLS: Cell<u32> = const { Cell::new(0) };
}

#[derive(Clone, Copy, PartialEq)]
enum Reply {
    Allow,
    Block,
    Ask,
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        SHELL_INSTALLS.with(|n| n.set(n.get() + 1));
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// The page with something worth keeping.
struct Editor;

impl Page for Editor {
    type Params = ();
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        ctx.on_leave(|| match ANSWER.with(|answer| answer.get()) {
            Reply::Allow => Verdict::Allow,
            Reply::Block => Verdict::Block,
            Reply::Ask => {
                let decision = Decision::new();
                ASKED.with(|slot| *slot.borrow_mut() = Some(decision.clone()));
                Verdict::Ask(Ask::new("Unsaved changes", "Discard", "Stay"), decision)
            }
        });
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Elsewhere;

impl Page for Elsewhere {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct ThirdPlace;

impl Page for ThirdPlace {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const EDITOR: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Editor>()];
const ELSEWHERE: [SegmentEntry<Headless>; 2] =
    [layout_entry::<Shell>(), segment_entry::<Elsewhere>()];
const THIRD: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<ThirdPlace>()];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Route {
    Editor,
    Elsewhere,
    Third,
}

impl RouteChain<Headless> for Route {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        match self {
            Route::Editor => &EDITOR,
            Route::Elsewhere => &ELSEWHERE,
            Route::Third => &THIRD,
        }
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        vec![Box::new(()), Box::new(())]
    }

    fn name(&self) -> &'static str {
        match self {
            Route::Editor => "Editor",
            Route::Elsewhere => "Elsewhere",
            Route::Third => "Third",
        }
    }
}

fn opened() -> Rc<Router<Headless>> {
    ANSWER.with(|answer| answer.set(Reply::Allow));
    ASKED.with(|slot| *slot.borrow_mut() = None);
    SHELL_INSTALLS.with(|n| n.set(0));

    let token = UiThreadToken::dangerously_create_token_unchecked();
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached(token)));
    router.navigate(Route::Editor).expect("navigate");
    router
}

fn go(router: &Rc<Router<Headless>>, route: Route) -> Navigation {
    router.navigate(route).expect("navigate")
}

fn on_editor(router: &Router<Headless>) -> bool {
    router.current_route::<Route>() == Some(Route::Editor)
}

#[test]
fn a_guard_that_allows_costs_nothing() {
    let router = opened();
    let outcome = go(&router, Route::Elsewhere);

    assert!(outcome.is_done());
    assert!(!on_editor(&router));
    assert!(router.pending().is_none(), "nothing was asked");
}

#[test]
fn a_guard_that_refuses_leaves_everything_where_it_was() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Block));

    let outcome = go(&router, Route::Elsewhere);

    assert!(matches!(outcome, Navigation::Blocked));
    assert!(on_editor(&router), "the route did not move");
    assert_eq!(
        SHELL_INSTALLS.with(|n| n.get()),
        1,
        "nothing was torn down and nothing was rebuilt - deciding mutates nothing"
    );
}

#[test]
fn a_question_parks_the_navigation_without_moving_anything() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Ask));

    let outcome = go(&router, Route::Elsewhere);

    assert!(matches!(outcome, Navigation::Deferred));
    assert_eq!(
        router.pending(),
        Some(Ask::new("Unsaved changes", "Discard", "Stay")),
        "the question is router state, which is what lets every backend draw it"
    );
    assert!(on_editor(&router), "still where it was");
}

#[test]
fn answering_yes_finishes_the_navigation() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Ask));
    go(&router, Route::Elsewhere);

    router.answer(true);

    assert!(!on_editor(&router));
    assert!(router.pending().is_none());
}

#[test]
fn answering_no_drops_it() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Ask));
    go(&router, Route::Elsewhere);

    router.answer(false);

    assert!(on_editor(&router));
    assert!(router.pending().is_none());
    assert_eq!(
        SHELL_INSTALLS.with(|n| n.get()),
        1,
        "the whole tree stayed up while the question was open"
    );
}

#[test]
fn a_second_navigation_supersedes_the_question() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Ask));
    go(&router, Route::Elsewhere);

    // The user changed their mind while the dialog was up. Last intent wins,
    // and the page is asked again about the new destination.
    let outcome = go(&router, Route::Third);
    assert!(matches!(outcome, Navigation::Deferred));

    router.answer(true);
    assert_eq!(
        router.current_route::<Route>(),
        Some(Route::Third),
        "the answer belongs to the navigation that was actually asked about"
    );
}

#[test]
fn an_answer_to_a_superseded_question_does_nothing() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Ask));
    go(&router, Route::Elsewhere);

    // The stale dialog's own token, kept from before the supersession.
    let stale = ASKED.with(|slot| slot.borrow().clone()).expect("a question");

    ANSWER.with(|answer| answer.set(Reply::Allow));
    go(&router, Route::Third);

    stale.allow();

    assert_eq!(
        router.current_route::<Route>(),
        Some(Route::Third),
        "a dialog answered after it was replaced must not navigate anywhere"
    );
}

#[test]
fn a_guard_may_answer_before_it_returns() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Ask));

    // Something that already knows - a cached answer, a policy, an actor that
    // had replied. The token is settled before the router ever sees it.
    let outcome = router
        .navigate_then(Route::Elsewhere, |_| {})
        .expect("navigate");
    ASKED
        .with(|slot| slot.borrow().clone())
        .expect("a question")
        .allow();

    assert!(matches!(outcome, Navigation::Deferred));
    assert!(!on_editor(&router), "it went through as soon as it could");
}

#[test]
fn only_the_scopes_that_are_leaving_are_asked() {
    let router = opened();
    ANSWER.with(|answer| answer.set(Reply::Block));

    // The same route again: nothing is leaving, so nothing is asked - and a
    // page that blocks everything would otherwise pin the application.
    let outcome = go(&router, Route::Editor);

    assert!(outcome.is_done(), "a segment does not guard against itself");
}
