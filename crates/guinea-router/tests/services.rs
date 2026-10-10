//! What a router hands out to code that has nothing else.

use guinea_app::app::GuineaApp;
use guinea_app::feature::FeatureHost;
use guinea_app::services::Services;
use guinea_router::headless::Headless;
use guinea_router::router::Router;

struct Store(&'static str);

#[test]
fn a_router_hands_out_what_the_application_provided() {
    let runtime = GuineaApp::new()
        .provide(Store("db"))
        .install()
        .expect("install");
    let router = Router::<Headless>::new(FeatureHost::under(&runtime.context()));

    let store = router.try_require::<Store>().map(|store| store.0);

    assert_eq!(store, Some("db"));
}
