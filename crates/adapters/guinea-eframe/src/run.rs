//! Installing the application into the loop eframe owns.
//!
//! eframe calls two things per frame: `logic`, before any drawing and even
//! while the window is hidden, and `ui`, when there is something to draw. That
//! split lands well here - the queue actors filled is drained in the first,
//! the route tree is drawn in the second.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::app::{GuineaApp, Stop, install_runtime, shutdown_current};
use guinea_app::feature::ScopeContext;
use guinea_core::actor::UiThreadToken;
use guinea_router::router::{NavigateHandle, RouteChain, RouteSink, Router};

use crate::{Egui, dispatcher, nav};

/// What [`run`] calls the root it opens.
pub const MAIN: &str = "main";

/// Runs the application in a window eframe opens, starting where `initial`
/// says.
///
/// ```ignore
/// guinea_eframe::run(app, "Processes", eframe::NativeOptions::default(), initial_route)
/// ```
///
/// A closure rather than a value because where an application starts is often
/// something only the installed plugins know - a route saved by the last run,
/// read out of the store the store plugin just provided. Called once, after
/// `install`, before the first frame, with the application's context to ask
/// them through.
pub fn run<R>(
    app: GuineaApp,
    title: &str,
    options: eframe::NativeOptions,
    initial: impl FnOnce(&ScopeContext) -> R,
) -> anyhow::Result<()>
where
    R: RouteChain<Egui> + Clone + PartialEq + 'static,
{
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread.
    dispatcher::install();

    // Genuinely this thread: it is the one that will draw, and nothing else
    // touches the router or the scopes.
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let runtime = match app.install(token.clone()) {
        Ok(runtime) => runtime,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };
    let initial = initial(&runtime.context());
    install_runtime(runtime);

    let router = Rc::new(Router::<Egui>::new(token));
    guinea_app::app::roots::set_label(router.root(), MAIN);

    let route = Rc::new(RefCell::new(initial.clone()));
    nav::install(NavigateHandle::new(router.clone(), {
        let route = route.clone();
        RouteSink::new(move |next: R| *route.borrow_mut() = next)
    }));

    router.navigate(initial.clone())?;

    let front = Frontend {
        router: router.clone(),
    };
    let outcome = eframe::run_native(
        title,
        options,
        Box::new(|cc| {
            // Now there is a context to wake: egui sleeps between frames, and
            // work finished on another thread has to ask for one.
            dispatcher::wake_with(cc.egui_ctx.clone());
            Ok(Box::new(front))
        }),
    );

    dispatcher::forget_waker();
    nav::clear();
    router.deactivate();
    shutdown_current();

    outcome.map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

struct Frontend {
    router: Rc<Router<Egui>>,
}

impl eframe::App for Frontend {
    /// Before the drawing, and also while the window is hidden: an actor that
    /// finished work still gets its turn on this thread.
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        guinea_core::devtools::profiling::frame_done();
        dispatcher::drain();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let asking = self.router.pending();

        // `add_enabled_ui(false, ..)` is the obligation the adapter contract
        // puts on every backend that draws over its own frame: while a guard's
        // question is up, the tree underneath must not take input, or the tabs
        // keep switching behind the dialog.
        {
            let _drawing = self.router.drawing();
            ui.add_enabled_ui(asking.is_none(), |ui| self.router.render(&()).draw(ui));
        }

        // The drawing is over and has let go of the chain, so a navigation
        // from inside it can now tear that chain down - which is the only
        // moment at which it can, and the reason it waited.
        match self.router.settle() {
            Ok(true) => ui.ctx().request_repaint(),
            Ok(false) => {}
            Err(error) => tracing::error!(%error, "a navigation asked for while drawing failed"),
        }

        if let Some(ask) = asking {
            question(ui, &ask, &self.router);
        }
    }
}

fn question(ui: &mut egui::Ui, ask: &guinea_core::guard::Ask, router: &Router<Egui>) {
    egui::Window::new("?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(&ask.text);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(&ask.cancel).clicked() {
                    router.answer(false);
                }
                if ui.button(&ask.confirm).clicked() {
                    router.answer(true);
                }
            });
        });
}
