//! Installing the application into a window Slint owns.
//!
//! The loop belongs to Slint, the way it belongs to the reactor under WinUI
//! and to us under ratatui. What is left here is the wiring: the dispatcher,
//! the runtime, the router, and telling the window which route it is showing.

use std::cell::RefCell;
use std::rc::Rc;

use std::sync::Arc;

use guinea_app::app::roots::RootId;
use guinea_app::app::windows::{SavedGeometry, WindowService, Windows};
use guinea_app::app::{GuineaApp, Stop, install_runtime, shutdown_current};
use guinea_app::feature::ScopeContext;
use guinea_core::actor::UiThreadToken;
use guinea_router::router::{NavigateHandle, RouteChain, RouteSink, Router};
use slint::ComponentHandle;

use crate::windows::SlintWindows;
use crate::{Slint, dispatcher, nav, question, root, windows};

/// What [`run`] calls the root it opens.
pub const MAIN: &str = "main";

/// Runs `window`, starting where `initial` says.
///
/// `show` is called with every route the application arrives at, and does the
/// one thing only the application can: point its own tree at the right branch.
///
/// `initial` is a closure rather than a value because where an application
/// starts is often something only the installed plugins know - a route saved
/// by the last run, read out of the store the store plugin just provided.
/// Called once, after `install`, before the first frame, with the
/// application's context to ask them through.
///
/// ```ignore
/// guinea_slint::run(app, AppWindow::new()?, initial_route, |window, route| {
///     window.set_page(match route {
///         Route::Processes { .. } => 0,
///         Route::Services { .. } => 1,
///         Route::Metrics { .. } => 2,
///     })
/// })
/// ```
pub fn run<R, W, S>(
    app: GuineaApp,
    window: W,
    initial: impl FnOnce(&ScopeContext) -> R,
    show: S,
) -> anyhow::Result<()>
where
    R: RouteChain<Slint> + Clone + PartialEq + 'static,
    W: ComponentHandle + 'static,
    S: Fn(&W, &R) + 'static,
{
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread.
    dispatcher::install();

    // Before the first route is installed: a page reaches for its globals
    // while it is being wired, and those hang off this window.
    root::install(window.clone_strong());

    // Provided before the plugins are built, so one that wants to restore a
    // window's geometry finds the service already there.
    let shell = Arc::new(SlintWindows::default());
    let app = app.provide(WindowService::from_arc(shell.clone()));

    // Genuinely this thread: it owns the window, and nothing else touches the
    // router or the scopes.
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let runtime = match app.install(token.clone()) {
        Ok(runtime) => runtime,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };
    let context = runtime.context();
    install_runtime(runtime);

    let router = Rc::new(Router::<Slint>::new(token));

    // The window belongs to this root, and stops belonging to it below, when
    // the loop is over.
    let root_id = router.root();
    shell.attach(root_id, &window);

    // Named, so that anything remembering something about this window between
    // runs has a key that outlives the id. One window per `run`, so it is the
    // main one; an application opening more names them itself.
    guinea_app::app::roots::set_label(root_id, MAIN);
    restore(&shell, root_id);
    let _watching = windows::watch(shell.clone());

    let initial = initial(&context);
    let window = Rc::new(window);
    let show = Rc::new(show);
    let route = Rc::new(RefCell::new(initial.clone()));

    let nav_handle = NavigateHandle::new(router.clone(), {
        let route = route.clone();
        let window = window.clone();
        let show = show.clone();
        RouteSink::new(move |next: R| {
            show(&window, &next);
            *route.borrow_mut() = next;
        })
    });
    nav::install(nav_handle);
    let _question = question::install(&router);

    // Installing the chain is what wires it - there is nothing to render.
    router.navigate(initial.clone())?;
    show(&window, &initial);

    let outcome = window.run();

    shell.detach(root_id);
    question::clear();
    nav::clear();
    router.deactivate();
    root::clear();
    shutdown_current();
    Ok(outcome?)
}

/// Puts the window back where it was last time, if anything remembers.
///
/// Before the loop starts, which is the whole point: a window restored after
/// it is on screen jumps, and a jump is what this is meant to avoid. Asks
/// rather than waits to be told - `RootOpened` travels through the UI queue
/// and would arrive too late.
fn restore(shell: &SlintWindows, root: RootId) {
    let Some(label) = guinea_app::app::roots::label(root) else {
        return;
    };
    let Some(saved) = guinea_app::app::app_services().get::<SavedGeometry>() else {
        return;
    };
    let Some(geometry) = saved.for_label(&label) else {
        return;
    };

    tracing::debug!(%root, %label, ?geometry, "restoring window");
    if shell.apply(root, geometry).is_err() {
        tracing::debug!(%root, "the window would not take the saved geometry");
    }
}
