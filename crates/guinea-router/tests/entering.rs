//! Guards on the way in, declared in the tree.
//!
//! The counterpart of `guards.rs`, which is about leaving. The two are not
//! symmetric and the tests are not either: a leave guard reads the scope it
//! belongs to, an enter guard answers before that scope exists.

use std::cell::Cell;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::guard::{Ask, Decision, Verdict};
use guinea_router::enter::{Enter, EnterCx};
use guinea_router::headless::{HeadlessCx, Layout, Page};
use guinea_router::router::{Navigation, Router};
use guinea_macros::routes;

thread_local! {
    /// What the guards answer, and who was asked, in order.
    static ADMIT: Cell<bool> = const { Cell::new(true) };
    static ASKED: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    /// The token of a parked enter question, so a test can answer it.
    static PARKED: std::cell::RefCell<Option<Decision>> = const {
        std::cell::RefCell::new(None)
    };
    /// Whether `Home` has unsaved work to ask about.
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    /// Set when a leave guard was consulted at all.
    static LEAVE_ASKED: Cell<bool> = const { Cell::new(false) };
}

fn asked(who: &'static str) {
    ASKED.with(|seen| seen.borrow_mut().push(who));
}

fn trail() -> Vec<&'static str> {
    ASKED.with(|seen| seen.borrow().clone())
}

/// On the whole area. Cascades to everything below.
struct RequiresSession;

impl Enter for RequiresSession {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        asked("RequiresSession");
        if ADMIT.with(Cell::get) {
            Verdict::Allow
        } else {
            Verdict::Block
        }
    }
}

/// On one page inside the area, so the order between the two is observable.
struct RequiresAdmin;

impl Enter for RequiresAdmin {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        asked("RequiresAdmin");
        Verdict::Allow
    }
}

/// Asks rather than answers, which is the point of `Decision`: the token goes
/// wherever the answer will come from, and the navigation waits.
struct AsksFirst;

impl Enter for AsksFirst {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        asked("AsksFirst");
        let verdict = Verdict::ask(Ask::new("Really?", "Yes", "No"));
        if let Verdict::Ask(_, decision) = &verdict {
            PARKED.with(|parked| *parked.borrow_mut() = Some(decision.clone()));
        }
        verdict
    }
}

struct Shell;

impl Layout for Shell {
    type Params = ShellParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

/// Outside the guarded area, and the page a test starts on.
struct Home;

impl Page for Home {
    type Params = HomeParams;
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &HomeParams) -> anyhow::Result<()> {
        // A question on the way out, to prove it is never reached when the
        // destination refuses. Off unless a test asks for it, so the other
        // tests navigate away without one.
        ctx.on_leave(|| {
            LEAVE_ASKED.with(|asked| asked.set(true));
            if DIRTY.with(Cell::get) {
                Verdict::ask(Ask::new("Unsaved work", "Discard", "Stay"))
            } else {
                Verdict::Allow
            }
        });
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Audit;

impl Page for Audit {
    type Params = AuditParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &AuditParams) -> anyhow::Result<()> {
        asked("Audit installed");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Inside the guarded area and opting out by name.
struct Public;

impl Page for Public {
    type Params = PublicParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &PublicParams) -> anyhow::Result<()> {
        asked("Public installed");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Confirmed;

impl Page for Confirmed {
    type Params = ConfirmedParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ConfirmedParams) -> anyhow::Result<()> {
        asked("Confirmed installed");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        page(Home)

        layout(Shell) guard(RequiresSession) {
            page(Audit) guard(RequiresAdmin) link("/admin/audit")
            page(Public) !guard(RequiresSession) link("/admin/public")
            page(Confirmed) guard(AsksFirst)
        }
    }
}

fn router() -> Rc<Router<Headless>> {
    ADMIT.with(|admit| admit.set(true));
    DIRTY.with(|dirty| dirty.set(false));
    ASKED.with(|seen| seen.borrow_mut().clear());
    LEAVE_ASKED.with(|asked| asked.set(false));
    PARKED.with(|parked| *parked.borrow_mut() = None);

    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    router.navigate(Route::Home {}).expect("the first route");

    // The leave guard belongs to `Home`, and only what happens after it is
    // installed is interesting.
    ASKED.with(|seen| seen.borrow_mut().clear());
    LEAVE_ASKED.with(|asked| asked.set(false));
    router
}

use guinea_router::headless::Headless;

#[test]
fn a_guard_on_a_layout_stands_in_front_of_the_pages_under_it() {
    let router = router();
    ADMIT.with(|admit| admit.set(false));

    let outcome = router.navigate(Route::Audit {}).expect("navigate");

    assert!(matches!(outcome, Navigation::Blocked));
    assert!(
        !trail().contains(&"Audit installed"),
        "nothing may be installed by a navigation that was refused: {:?}",
        trail()
    );
}

#[test]
fn an_area_answers_before_a_page_inside_it() {
    let router = router();
    router.navigate(Route::Audit {}).expect("navigate");

    assert_eq!(
        trail(),
        ["RequiresSession", "RequiresAdmin", "Audit installed"],
        "outermost first: an area refuses before an inner page considers anything"
    );
}

#[test]
fn opting_out_by_name_opens_the_page() {
    let router = router();
    ADMIT.with(|admit| admit.set(false));

    let outcome = router.navigate(Route::Public {}).expect("navigate");

    assert!(matches!(outcome, Navigation::Done(_)));
    assert_eq!(
        trail(),
        ["Public installed"],
        "the guard it dropped was not even asked"
    );
}

#[test]
fn a_refused_destination_is_never_a_question_about_unsaved_work() {
    // `Home` would ask on the way out. Asking "discard changes?" and then
    // refusing anyway is a worse answer than refusing, so entering is decided
    // first and leaving is never reached.
    let router = router();
    DIRTY.with(|dirty| dirty.set(true));
    ADMIT.with(|admit| admit.set(false));

    let outcome = router.navigate(Route::Audit {}).expect("navigate");

    assert!(matches!(outcome, Navigation::Blocked));
    assert!(
        !LEAVE_ASKED.with(Cell::get),
        "the leave guard was consulted for a navigation that could not happen"
    );
    assert!(router.pending().is_none(), "and nothing was put on screen");
}

#[test]
fn an_enter_guard_may_ask_and_the_navigation_waits() {
    let router = router();

    let outcome = router.navigate(Route::Confirmed {}).expect("navigate");

    assert!(matches!(outcome, Navigation::Deferred));
    assert!(router.pending().is_some(), "the question is router state");
    assert!(
        !trail().contains(&"Confirmed installed"),
        "deciding mutates nothing"
    );

    let decision = PARKED.with(|parked| parked.borrow_mut().take()).expect("a token");
    decision.allow();

    assert!(trail().contains(&"Confirmed installed"));
    assert!(router.pending().is_none());
}

#[test]
fn the_manifest_says_what_stands_in_front_of_each_address() {
    let surface = Route::deep_links();

    let audit = surface
        .iter()
        .find(|link| link.path == "/admin/audit")
        .expect("the guarded address");
    assert_eq!(audit.guards, ["RequiresSession", "RequiresAdmin"]);

    let public = surface
        .iter()
        .find(|link| link.path == "/admin/public")
        .expect("the opted-out address");
    assert_eq!(
        public.guards,
        [] as [&str; 0],
        "an address that dropped its guard says so where it is reviewed"
    );
}
