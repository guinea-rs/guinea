//! The terminal front end. Same application as `processes-app`: same actors,
//! same reducers, same features - only the pages and the route tree differ.

mod cursor;
mod layouts;
mod pages;
mod routes;

use guinea::app::GuineaApp;
use guinea::services::Services as _;
use guinea::ratatui::{Flow, Tui, pressed, run};
use guinea_router::router::Router;
use ratatui::crossterm::event::{Event, KeyCode};
use routes::Route;

use guinea_plugin_store::Store;
use processes_core::startup;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Where the store keeps the route between runs.
const LAST_ROUTE: &str = "route";

/// Where the last run left off, or the first tab.
///
/// Runs after the plugins are installed, which is the whole reason `run` takes
/// a closure: the store does not exist until the store plugin provides it.
///
/// Every failure ends here rather than propagating. A saved route outlives the
/// build that wrote it, so one that no longer parses is an ordinary thing to
/// find on the way in - the application starts where it always did.
fn initial_route(store: Option<&Store>) -> Route {
    store
        .and_then(|store| store.kv().get::<String>(LAST_ROUTE).ok().flatten())
        .as_deref()
        .and_then(Route::restore)
        .unwrap_or(Route::Processes {
            context: "ubuntu".to_string(),
        })
}

/// Writes the route down, if it agreed to survive a restart.
///
/// The router hands over a string and has no opinion about where it goes -
/// this is the half that does.
fn remember(store: &Store, route: &Route) {
    let Some(saved) = route.save() else {
        return;
    };

    if let Err(error) = store.kv().set(LAST_ROUTE, &saved) {
        tracing::warn!(%error, "the route could not be remembered");
    }
}

fn main() -> anyhow::Result<()> {
    // To a file: stdout is the drawing surface, and a log line in the middle
    // of a frame corrupts it.
    let log = std::fs::File::create("processes-tui.log")?;
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
                // JSON, so both front ends can run at once: redb locks its
                // file and the second one would refuse to start.
                .backend(guinea_plugin_store::amethystate::store::builder::Backend::Json),
        )
        .plugin(guinea_plugin_l10n::L10nPlugin::<processes_core::l10n::L10n>::new("en"))
        .feature(startup::Startup);

    run(
        app,
        |cx| initial_route(cx.try_require::<Store>().as_deref()),
        on_key,
    )
}

/// The keys no page took: the ones that are the application's, not a page's.
fn on_key(
    event: &Event,
    nav: &guinea_router::router::NavigateHandle<Tui, Route>,
    router: &Router<Tui>,
) -> Flow {
    let Some(code) = pressed(event) else {
        return Flow::Continue;
    };

    let context = router
        .current_route::<Route>()
        .map(|route| route.context().to_string())
        .unwrap_or_default();

    match code {
        KeyCode::Char('q') => return Flow::Exit,
        // Back where it came from, and only quit when there is nowhere left -
        // the jest every terminal user tries first.
        KeyCode::Esc => {
            if !nav.back() {
                return Flow::Exit;
            }
        }
        KeyCode::Char('1') => nav.to(Route::Processes { context }),
        KeyCode::Char('2') => nav.to(Route::Services { context }),
        KeyCode::Char('3') => nav.to(Route::Metrics { context }),
        _ => {}
    }

    // One place rather than beside every `nav.to`: whatever the key did to the
    // route - including `back` - is where the next run should start.
    if let (Some(route), Some(store)) = (
        router.current_route::<Route>(),
        router.try_require::<Store>(),
    ) {
        remember(&store, &route);
    }

    Flow::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea::app::TestApp;

    /// The two halves of `restorable` against a real store, since separately
    /// they both pass while agreeing about nothing.
    #[test]
    fn a_saved_route_is_where_the_next_run_starts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = TestApp::new();
        app.install(guinea_plugin_store::StorePlugin::at(dir.path().join("store")))
            .expect("the store plugin");
        let store = app.require::<Store>().expect("the store");

        assert!(
            matches!(initial_route(Some(&store)), Route::Processes { .. }),
            "nothing was saved yet, so the application starts where it always did"
        );

        remember(
            &store,
            &Route::Metrics {
                context: "fedora".to_string(),
            },
        );

        assert_eq!(
            initial_route(Some(&store)),
            Route::Metrics {
                context: "fedora".to_string()
            }
        );

        // A saved route outlives the build that wrote it, and one that no
        // longer exists is an ordinary thing to find on the way in.
        store
            .kv()
            .set(LAST_ROUTE, &r#"{"route":"Removed","fields":{}}"#.to_string())
            .expect("set");

        assert!(matches!(initial_route(Some(&store)), Route::Processes { .. }));
    }
}
