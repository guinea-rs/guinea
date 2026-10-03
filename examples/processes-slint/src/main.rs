//! The Slint front end. Same application as `processes-app` and
//! `processes-tui`: same actors, same reducers, same features - only the
//! components and the route tree differ.

mod layouts;
mod pages;
mod routes;

/// Compiled from `app.slint` and everything it imports, including the route
/// tree generated from `routes.rs`.
mod ui {
    slint::include_modules!();
}

use crate::ui::RouteId;

// `route_id`, generated from the same declaration as the tree above.
include!(concat!(env!("OUT_DIR"), "/route_id.rs"));

use guinea::app::GuineaApp;
use guinea::slint::run;
use routes::Route;

use processes_core::startup;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::ui::AppWindow;

fn initial_route() -> Route {
    Route::Processes {
        context: "ubuntu".to_string(),
    }
}

fn main() -> anyhow::Result<()> {
    // To a file, like the other two front ends: a windowed application has no
    // console to watch, and its stdout is block buffered.
    let log = std::fs::File::create("processes-slint.log")?;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,guinea=debug,processes_core=debug".into()),
        )
        .with(guinea_core::trace::json(log))
        .init();

    let app = GuineaApp::new()
        .meta(guinea::app::AppMeta::new(
            "Processes",
            "dev.uniproc.guinea.processes",
            env!("CARGO_PKG_VERSION"),
            "uniproc",
        ))
        .plugin(
            guinea_plugin_store::StorePlugin::for_app("guinea-processes-app-example", "settings")
                // JSON, so every front end can run at once: redb locks its
                // file and the second one would refuse to start.
                .backend(guinea_plugin_store::amethystate::store::builder::Backend::Json),
        )
        .plugin(guinea_plugin_l10n::L10nPlugin::<processes_core::l10n::L10n>::new("en"))
        // Opens the window where it was left, and keeps it that way.
        .plugin(guinea_plugin_window_state::WindowStatePlugin::new())
        .feature(startup::Startup);

    run(app, AppWindow::new()?, |_| initial_route(), |window, route| {
        window.set_route(route_id(route))
    })
}
