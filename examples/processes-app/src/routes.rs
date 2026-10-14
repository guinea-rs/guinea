use guinea_macros::routes;

use crate::pages::metrics::Metrics;
use crate::pages::processes::Processes;
use crate::pages::services::Services;
use crate::layouts::tabs::TabsLayout;

routes! {
    Route {
        layout(TabsLayout) {
            page(Processes) { context: String }
            page(Services) { context: String }
            page(Metrics) link("/:context/metrics") { context: String }
        }
    }
}

/// What this application answers to from outside, committed and diffed.
///
/// The scheme comes from `app.toml` rather than from a literal here: it is the
/// same identity the installer registers, and a manifest that agreed with a
/// second copy of it would only be agreeing with itself.
#[test]
fn deep_links_are_what_was_shipped() {
    guinea::manifest::check(
        "deeplinks.toml",
        guinea::app_meta!().identifier,
        &[&Route::deep_links()],
    );
}
