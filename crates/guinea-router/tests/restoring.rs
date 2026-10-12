//! Routes that survive a restart.
//!
//! The tier that makes the compiler prove something. `link` says an address
//! exists; `restorable` says the route can be rebuilt from text alone, and the
//! generated code is the proof - a field that cannot make the round trip fails
//! to build at its own declaration.
//!
//! What is deliberately absent is storage. The router hands over a string and
//! takes one back; where it lives between runs is the application's business,
//! the same way drawing is the backend's.

use guinea_app::feature::FeatureInitContext;
use guinea_macros::routes;
use guinea_router::headless::{HeadlessCx, Layout, Page};

struct Session;

impl Layout for Session {
    type Params = SessionParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &SessionParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Processes;

impl Page for Processes {
    type Params = ProcessesParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ProcessesParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Metrics;

impl Page for Metrics {
    type Params = MetricsParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &MetricsParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Outside the restorable area, so a tree can hold both kinds.
struct Splash;

impl Page for Splash {
    type Params = SplashParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &SplashParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Session) restorable {
            page(Processes) link("/:context/processes") { context: String }
            page(Metrics) { context: String, window: u32 }
        }

        page(Splash) { after: String }
        page(Kind) restorable { r#type: String }
    }
}

/// A field named like a keyword.
struct Kind;

impl Page for Kind {
    type Params = KindParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &KindParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

#[test]
fn a_field_named_like_a_keyword_is_saved_under_its_own_name() {
    let route = Route::Kind {
        r#type: "disk".to_string(),
    };

    let saved = route.save().expect("it agreed to survive a restart");
    assert!(saved.contains(r#""type":"disk""#), "{saved}");
    assert_eq!(Route::restore(&saved), Some(route));
}

#[test]
fn a_restorable_route_comes_back_whole() {
    let route = Route::Processes {
        context: "ubuntu".to_string(),
    };

    let saved = route.save().expect("it agreed to survive a restart");
    assert_eq!(Route::restore(&saved), Some(route));
}

#[test]
fn every_field_comes_back_including_the_ones_no_address_carries() {
    // `Metrics` has no `link`, so nothing about it is in a path - and it is
    // still restorable, which is the whole point of the two tiers being
    // separate.
    let route = Route::Metrics {
        context: "fedora".to_string(),
        window: 300,
    };

    let saved = route.save().expect("restorable through its layout");
    assert_eq!(Route::restore(&saved), Some(route));
}

#[test]
fn a_route_that_did_not_agree_writes_nothing() {
    let route = Route::Splash {
        after: "install".to_string(),
    };

    assert_eq!(route.save(), None, "and there is nothing to read back");
}

#[test]
fn the_tree_says_whether_any_of_it_is_worth_keeping() {
    assert!(
        Route::RESTORABLE,
        "an application asks this before it keeps a saved route at all"
    );
}

#[test]
fn what_an_older_build_wrote_is_read_as_nothing() {
    // A saved session outlives the version that made it. A route that was
    // renamed, a field that changed type, a file that is not one of ours - all
    // ordinary, and all `None` rather than an error.
    assert_eq!(Route::restore("{}"), None);
    assert_eq!(Route::restore(r#"{"route":"Removed","fields":{}}"#), None);
    assert_eq!(
        Route::restore(r#"{"route":"Metrics","fields":{"context":"x","window":"soon"}}"#),
        None,
        "the field changed type"
    );
    assert_eq!(
        Route::restore(r#"{"route":"Metrics","fields":{"context":"x"}}"#),
        None,
        "a field was added since"
    );
}

#[test]
fn saving_is_stable() {
    // What is written has to be comparable with what was written last time,
    // or an application that only stores on change stores on every run.
    let route = Route::Metrics {
        context: "fedora".to_string(),
        window: 300,
    };

    assert_eq!(route.save(), route.clone().save());
}

#[test]
fn the_manifest_says_which_addresses_outlive_the_process() {
    let surface = Route::deep_links();
    let processes = surface
        .iter()
        .find(|link| link.path == "/:context/processes")
        .expect("the address");

    assert!(
        processes.restorable,
        "a route written to disk between runs is a fact about the outside"
    );
}
