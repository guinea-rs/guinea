use guinea_macros::routes;

use crate::pages::metrics::Metrics;
use crate::pages::processes::Processes;
use crate::pages::services::Services;
use crate::layouts::tabs::TabsLayout;

// The same tree as the WinUI front end, aimed at the other backend. The pages
// differ; where they sit and what installs with them does not.
//
// Only `Metrics` carries a `link`: an address is a promise to whoever holds it
// outside the application, and two of these three pages have made none.
routes! {
    backend = guinea::ratatui::Tui,
    Route {
        layout(TabsLayout) restorable {
            page(Processes) { context: String }
            page(Services) { context: String }
            page(Metrics) link("/:context/metrics") { context: String }
        }
    }
}

impl Route {
    /// What every tab was reached with.
    pub fn context(&self) -> &str {
        match self {
            Route::Processes { context }
            | Route::Services { context }
            | Route::Metrics { context } => context,
        }
    }
}

/// What this application answers to from outside, committed and diffed.
///
/// Not a test of the router - a test of this application's promises. It fails
/// when the external surface changes, which is the moment to notice, and
/// `GUINEA_BLESS=1` rewrites the file once the change is meant.
#[test]
fn deep_links_are_what_was_shipped() {
    guinea::manifest::check("deeplinks.toml", "guinea-processes", &[Route::deep_links()]);
}
