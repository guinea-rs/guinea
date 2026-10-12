//! What a segment that captures nothing is handed: `()`, the same thing a
//! segment that declares no `Params` defaults to.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_macros::routes;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page};
use guinea_router::router::Router;

pub struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

pub struct Home;

impl Page for Home {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Names the generated params, as code written before this did.
pub struct Named;

impl Page for Named {
    type Params = NamedParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &NamedParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

thread_local! {
    static HANDED: RefCell<Option<HashMap<String, u32>>> = const { RefCell::new(None) };
}

/// Captures a type whose generics hold a comma.
pub struct Totals;

impl Page for Totals {
    type Params = TotalsParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, params: &TotalsParams) -> anyhow::Result<()> {
        HANDED.with(|handed| *handed.borrow_mut() = Some(params.by_host.clone()));
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Shell) {
            page(Home) { }
            page(Named)
        }
        page(Totals) { by_host: HashMap<String, u32> }
    }
}

fn router() -> Rc<Router<Headless>> {
    Rc::new(Router::<Headless>::new(FeatureHost::detached()))
}

#[test]
fn a_page_and_a_layout_that_capture_nothing_are_handed_unit() {
    let navigated = router().navigate(Route::Home {}).map(|_| ());

    assert!(navigated.is_ok(), "{navigated:?}");
}

#[test]
fn naming_the_generated_params_still_works() {
    let navigated = router().navigate(Route::Named {}).map(|_| ());

    assert!(navigated.is_ok(), "{navigated:?}");
}

#[test]
fn a_field_whose_type_has_a_comma_in_its_generics_is_handed_whole() {
    let by_host = HashMap::from([("ubuntu".to_string(), 3)]);

    router()
        .navigate(Route::Totals { by_host: by_host.clone() })
        .expect("navigate");

    assert_eq!(HANDED.with(|handed| handed.borrow().clone()), Some(by_host));
}
