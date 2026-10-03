//! What a router hands out to code that has nothing else.

use guinea_app::feature::FeatureHost;
use guinea_app::services::Services;
use guinea_core::SharedState;
use guinea_core::actor::UiThreadToken;
use guinea_router::headless::Headless;
use guinea_router::router::Router;

struct Store(&'static str);

#[test]
fn a_router_hands_out_what_the_application_provided() {
    let services = SharedState::default();
    services.insert(Store("db"));
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let router = Router::<Headless>::with_host(FeatureHost::with_services(token, services));

    let store = router.try_require::<Store>().map(|store| store.0);

    assert_eq!(store, Some("db"));
}
