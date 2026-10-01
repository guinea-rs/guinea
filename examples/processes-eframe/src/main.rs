//! The egui front end. Same application as the other three: same actors, same
//! reducers, same features - only the drawing and the route tree differ.

mod layouts;
mod pages;
mod routes;

use guinea::app::GuineaApp;
use guinea::eframe::run;
use routes::Route;

use processes_core::startup;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn initial_route() -> Route {
    Route::Processes {
        context: "ubuntu".to_string(),
    }
}

fn main() -> anyhow::Result<()> {
    // To a file, like the other windowed front ends: there is no console to
    // watch, and stdout is block buffered.
    let log = std::fs::File::create("processes-eframe.log")?;
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
        .plugin(guinea_plugin_devtools::DevToolsPlugin::new())
        .feature(startup::Startup);

    run(
        app,
        "guinea · processes (egui)",
        eframe::NativeOptions::default(),
        initial_route,
    )
}
