//! A part: a page with no route of its own, mounted with the layout it is
//! written in and shown in that layout's slot.
//!
//! It lives as long as its layout does - pages come and go under it, and it
//! stays - and it sleeps when a kept layout sleeps.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use guinea_app::feature::{Feature, FeatureHost, FeatureInitContext};
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::{Reducer, Scope};
use guinea_macros::{routes, slot};
use guinea_router::construction::check;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page};
use guinea_router::router::{RouteChain, Router};

#[slot]
pub struct Footer;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Title(&'static str);

impl Reducer for Title {
    type Update = &'static str;

    fn reduce(&mut self, to: &'static str) {
        self.0 = to;
    }
}

pub struct Chrome;

impl Feature for Chrome {
    type Params = ();
    type Exports = (Title,);

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        cx.state::<Title>().seed(Title("guinea")).plain();
        Ok(Self)
    }
}

thread_local! {
    static INSTALLED: Cell<usize> = const { Cell::new(0) };
    static DROPPED: Cell<usize> = const { Cell::new(0) };
    static PART: Cell<Option<Scope>> = const { Cell::new(None) };
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// What the part installs - counted in and out.
pub struct Charting;

impl Feature for Charting {
    type Params = ();
    type Exports = ();

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        INSTALLED.set(INSTALLED.get() + 1);
        PART.set(Some(cx.scope));
        Ok(Self)
    }
}

impl Drop for Charting {
    fn drop(&mut self) {
        DROPPED.set(DROPPED.get() + 1);
    }
}

pub struct Charts;

impl Page for Charts {
    type Params = ();
    type Installs = Charting;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Charting> {
        ctx.install::<Charting>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        let (title, _) = cx.read::<Title>();
        SEEN.with(|seen| seen.borrow_mut().push(format!("charts read {}", title.0)));
    }
}

pub struct Shell;

impl Layout for Shell {
    type Params = ShellParams;
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.slot::<Footer>();
        cx.outlet();
    }
}

pub struct Kept;

impl Layout for Kept {
    type Params = KeptParams;
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &KeptParams) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.slot::<Footer>();
        cx.outlet();
    }
}

macro_rules! plain_page {
    ($page:ident, $params:ident) => {
        pub struct $page;

        impl Page for $page {
            type Params = $params;
            type Installs = ();

            fn install(_ctx: &FeatureInitContext, _params: &$params) -> anyhow::Result<()> {
                Ok(())
            }

            fn view(_cx: &mut HeadlessCx<Self>) {}
        }
    };
}

plain_page!(A, AParams);
plain_page!(B, BParams);
plain_page!(C, CParams);
plain_page!(Elsewhere, ElsewhereParams);

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Shell) {
            part(Charts) => Footer
            page(A)
            page(B)
        }
        layout(Kept) keep {
            part(Charts) => Footer
            page(C)
        }
        page(Elsewhere)
    }
}

fn router() -> Rc<Router<Headless>> {
    INSTALLED.set(0);
    DROPPED.set(0);
    PART.set(None);
    SEEN.with(|seen| seen.borrow_mut().clear());

    let token = UiThreadToken::dangerously_create_token_unchecked();
    Rc::new(Router::<Headless>::new(FeatureHost::detached(token)))
}

#[test]
fn a_part_installs_with_its_layout_and_stays_while_pages_change_under_it() {
    let router = router();

    router.navigate(Route::A {}).expect("to a");
    assert_eq!(INSTALLED.get(), 1, "installed with the shell");

    router.navigate(Route::B {}).expect("to b");
    assert_eq!((INSTALLED.get(), DROPPED.get()), (1, 0), "the shell stayed, and so did its part");
}

#[test]
fn a_part_goes_with_its_layout() {
    let router = router();

    router.navigate(Route::A {}).expect("to a");
    router.navigate(Route::Elsewhere {}).expect("elsewhere");

    assert_eq!(DROPPED.get(), 1);
}

#[test]
fn a_part_of_a_kept_layout_sleeps_and_wakes_with_it() {
    let router = router();

    router.navigate(Route::C {}).expect("to c");
    assert!(PART.get().is_some(), "the part installed with its layout");
    let part = PART.get().unwrap();
    assert!(part.is_awake());

    router.navigate(Route::Elsewhere {}).expect("elsewhere");
    assert!(part.is_alive() && !part.is_awake(), "asleep with its layout, not gone");

    router.navigate(Route::C {}).expect("back");
    assert!(part.is_awake(), "woken with its layout");
    assert_eq!(INSTALLED.get(), 1, "woken, not installed again");
}

#[test]
fn a_part_reads_what_its_layout_exports() {
    let router = router();

    router.navigate(Route::A {}).expect("to a");
    router.render(&());

    assert_eq!(SEEN.with(|seen| seen.borrow().clone()), ["charts read guinea"]);
}

mod doubled {
    use super::*;

    pub struct Greedy;

    impl Page for Greedy {
        type Params = ();
        type Installs = Chrome;

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
            ctx.install::<Chrome>(&())
        }

        fn view(_cx: &mut HeadlessCx<Self>) {}
    }

    plain_page!(D, DParams);

    routes! {
        backend = guinea_router::headless::Headless,
        Doubled {
            layout(Shell) {
                part(Greedy) => Footer
                page(D)
            }
        }
    }

    #[test]
    fn a_part_is_checked_with_the_tree() {
        let construction = check(<Doubled as RouteChain<Headless>>::tree());

        assert_eq!(
            construction.errors,
            ["`Chrome` is installed by `Shell`, and again by `Greedy` below it"]
        );
    }
}
