//! The WinUI front end. Everything that is not drawing lives in
//! `processes-core`, which the terminal front end links just the same.

mod layouts;
mod pages;
mod routes;

use routes::Route;

use guinea::app::GuineaApp;
use guinea::winui::{Window, run};
use processes_core::startup;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn initial_route() -> Route {
    Route::Processes {
        context: "ubuntu".to_string(),
    }
}

fn main() -> anyhow::Result<()> {
    // To a file, like the terminal front end: a windowed application has no
    // console to watch, and its stdout is block buffered - a line written now
    // would show up only when it exits.
    let log = std::fs::File::create("processes-app.log")?;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,guinea=debug,processes_core=debug".into()),
        )
        .with(guinea_core::trace::json(log))
        .with(guinea_core::devtools::layer())
        .init();

    if let Some(dll) = std::env::var_os("GUINEA_XAML_TAP") {
        std::thread::spawn(move || load_tap(dll.into()));
    }

    let app = GuineaApp::new()
        .meta(guinea::app_meta!())
        .plugin(
            guinea_plugin_store::StorePlugin::for_app("guinea-processes-app-example", "settings")
                // JSON, so both front ends can run at once: redb locks its
                // file and the second one would refuse to start.
                .backend(guinea_plugin_store::amethystate::store::builder::Backend::Json),
        )
        .plugin(guinea_plugin_l10n::L10nPlugin::<processes_core::l10n::L10n>::new("en"))
        .plugin(guinea_plugin_devtools::DevToolsPlugin::new())
        .feature(startup::Startup);

    run(
        app,
        Window::new()
            .title(guinea::app_meta!().window_title)
            .client_size(420.0, 420.0),
        |_| initial_route(),
    )
}

/// Loads the XAML tap into this very process once XAML is up, instead of
/// devtools injecting it from outside. An experiment.
fn load_tap(dll: std::path::PathBuf) {
    let pid = std::process::id();
    let copy = std::env::temp_dir().join(format!("guinea-xaml-tap-self-{pid}.dll"));
    if let Err(error) = std::fs::copy(&dll, &copy) {
        tracing::warn!(%error, "copying the tap");
        return;
    }

    for attempt in 1..=40 {
        match guinea_xaml_tap::inject::inject(pid, &copy) {
            Ok(()) => {
                tracing::info!(attempt, "the tap loaded into its own process");
                return;
            }
            Err(error) => {
                tracing::info!(attempt, %error, "the tap is not in yet");
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }

    tracing::warn!("the tap never loaded");
}
