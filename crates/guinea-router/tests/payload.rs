//! A route carrying something with no identity.
//!
//! The router asks one question about a parameter: is this still the same one?
//! For a channel, a callback, an `Arc<dyn Trait>` there is no answer - they
//! have an address and nothing else - and answering it anyway is what forces
//! `PartialEq` onto a payload that has no business implementing it.
//!
//! `~` says the question does not apply. The value still reaches `install`; it
//! is simply never compared and never kept, so the router cannot claim two
//! entries are the same and reinstalls every time.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_macros::routes;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page};
use guinea_router::router::{RouteChain, Router};

/// No `PartialEq`, no `Debug`, no `Default` - the shape of a real payload.
/// `Clone` only, because a route is cloned on the way in and a handle to
/// something shared is the case `~` exists for.
#[derive(Clone)]
pub struct Feed(Rc<Cell<u32>>);

thread_local! {
    /// Which feed each install saw, in order.
    static SEEN: std::cell::RefCell<Vec<u32>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

struct Shell;

impl Layout for Shell {
    type Params = ShellParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<()> {
        SEEN.with(|seen| seen.borrow_mut().push(0));
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Wizard;

impl Page for Wizard {
    type Params = WizardParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, params: &WizardParams) -> anyhow::Result<()> {
        SEEN.with(|seen| seen.borrow_mut().push(params.result.0.get()));
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Shell) {
            page(Wizard) { step: u8, result: ~Feed }
        }
    }
}

fn feed(id: u32) -> Feed {
    Feed(Rc::new(Cell::new(id)))
}

fn router() -> Rc<Router<Headless>> {
    SEEN.with(|seen| seen.borrow_mut().clear());

    Rc::new(Router::<Headless>::new(FeatureHost::detached()))
}

fn seen() -> Vec<u32> {
    SEEN.with(|seen| seen.borrow().clone())
}

#[test]
fn the_payload_reaches_install() {
    let router = router();
    router
        .navigate(Route::Wizard {
            step: 1,
            result: feed(7),
        })
        .expect("navigate");

    assert_eq!(seen(), [0, 7], "the layout, then the page with its feed");
}

#[test]
fn a_new_payload_is_a_new_thing() {
    // Same step, different feed. Nothing about the identity changed, and the
    // page still has to be rebuilt - otherwise it would quietly go on holding
    // the feed its caller replaced.
    let router = router();
    router
        .navigate(Route::Wizard {
            step: 1,
            result: feed(7),
        })
        .expect("navigate");
    router
        .navigate(Route::Wizard {
            step: 1,
            result: feed(9),
        })
        .expect("navigate again");

    assert_eq!(
        seen(),
        [0, 7, 9],
        "the page reinstalled with the new feed; the layout above it did not"
    );
}

#[test]
fn identity_is_what_it_says_it_is() {
    // Equality over the fields that have one. Two routes differing only in
    // their payload are the same route - which is the statement `~` makes.
    let one = Route::Wizard {
        step: 1,
        result: feed(7),
    };
    let other = Route::Wizard {
        step: 1,
        result: feed(9),
    };
    let later = Route::Wizard {
        step: 2,
        result: feed(7),
    };

    assert_eq!(one, other);
    assert_ne!(one, later);
}

#[test]
fn debug_prints_the_identity_and_says_there_is_more() {
    let route = Route::Wizard {
        step: 3,
        result: feed(7),
    };

    let printed = format!("{route:?}");
    assert!(printed.contains("Wizard"), "{printed}");
    assert!(printed.contains("step: 3"), "{printed}");
    assert!(
        printed.contains(".."),
        "a payload with no `Debug` is still worth admitting to: {printed}"
    );
}

#[test]
fn a_layout_never_derives_a_payload_from_its_pages() {
    // `ShellParams` is the intersection of what its pages carry, and a `~`
    // field is not in it: a layout's parameters exist to be compared, and this
    // one has nothing to contribute to that.
    let params: Vec<Box<dyn Any>> = RouteChain::<Headless>::params(&Route::Wizard {
        step: 1,
        result: feed(7),
    });

    assert!(
        params[0].downcast_ref::<ShellParams>().is_some(),
        "the layout still gets its own params struct"
    );
    assert!(params[1].downcast_ref::<WizardParams>().is_some());
}
