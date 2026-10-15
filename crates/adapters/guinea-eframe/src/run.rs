//! Installing the application into the loop eframe owns.
//!
//! eframe calls two things per frame: `logic`, before any drawing and even
//! while the window is hidden, and `ui`, when there is something to draw. That
//! split lands well here - the queue actors filled is drained in the first,
//! the window is drawn in the second.

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use guinea_app::app::{AppRuntime, GuineaApp, Stop};
use guinea_app::feature::FeatureHost;
use guinea_core::scope::Scope;
use guinea_core::ui_thread::UiLoop;
use guinea_router::mount::Pass;
use guinea_router::navigator::{Asking, Navigator, Routable, Watching};

use crate::{Mountable, drawn, failed};

/// What [`run`] calls the window it opens.
pub const MAIN: &str = "main";

/// A window: the layout it roots at, where it starts, and how it looks.
///
/// ```ignore
/// Window::of::<Shell>().title("guinea").at(Route::Processes)
/// ```
pub struct Window {
    title: String,
    options: eframe::NativeOptions,
    root: Root,
    start: Option<Box<dyn FnOnce(Scope) -> Rc<dyn Asking>>>,
}

/// The layout a window roots at.
pub(crate) struct Root {
    mount: fn(&Pass) -> anyhow::Result<Scope>,
}

impl Root {
    pub(crate) fn of<L: Mountable<Params = ()>>() -> Self {
        Self {
            mount: |pass| pass.mount::<L, ()>(&(), |cx| L::install(cx, &())),
        }
    }
}

impl Window {
    /// A window whose content is the layout or page `L`.
    pub fn of<L: Mountable<Params = ()>>() -> Self {
        Self {
            title: String::new(),
            options: eframe::NativeOptions::default(),
            root: Root::of::<L>(),
            start: None,
        }
    }

    /// Starts the window at `route`, with a navigator of its own over `R`:
    /// what the layouts in it find with `cx.navigate::<R>()`, and whose
    /// guards' questions the window puts up as a dialog.
    pub fn at<R: Routable>(mut self, route: R) -> Self {
        self.start = Some(Box::new(move |window| Rc::new(Navigator::open(window, route)) as Rc<dyn Asking>));
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// How eframe opens the window: its size, position, renderer.
    pub fn options(mut self, options: eframe::NativeOptions) -> Self {
        self.options = options;
        self
    }
}

/// One frame of the window whose scope is `window`: its root, mounted, and
/// drawn into `ui`.
pub(crate) fn frame(window: Scope, root: &Root, ui: &mut egui::Ui) {
    let pass = Pass::begin(window);
    let mounted = (root.mount)(&pass);
    pass.end();

    match mounted {
        Ok(root) => drawn(root).draw(ui),
        Err(error) => failed(&error).draw(ui),
    }
}

/// Installs `app`, opens the window `main` makes, and runs until it closes.
///
/// `main` runs after the plugins are installed, and is handed the installed
/// application to read what they provided from - where to start, say.
///
/// ```ignore
/// guinea_eframe::run(app, |_| Window::of::<Shell>().title("guinea").at(Route::Processes))
/// ```
///
/// A root that fails to mount ends it with why, before the window opens.
pub fn run(app: GuineaApp, main: impl FnOnce(&AppRuntime) -> Window) -> anyhow::Result<()> {
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread. egui sleeps between frames,
    // so work handed over asks for one - once there is a context to ask.
    let context: Arc<Mutex<Option<egui::Context>>> = Arc::default();
    let repaint = {
        let context = context.clone();
        move || {
            if let Some(context) = context.lock().ok().as_deref().and_then(Option::as_ref) {
                context.request_repaint();
            }
        }
    };
    let ui = UiLoop::install_waking(repaint.clone());

    let runtime = match app.start() {
        Ok(runtime) => runtime,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };
    let window = main(&runtime);

    let host = FeatureHost::under(&runtime.context());
    let scope = host.scope();
    guinea_app::app::roots::set_label(scope, MAIN);

    let pass = Pass::begin(scope);
    let mounted = (window.root.mount)(&pass);
    pass.end();
    if let Err(error) = mounted {
        return Err(error.context("guinea: mounting the main window"));
    }

    // A move made while drawing is drawn in the next frame, and egui makes
    // one only when asked.
    let asking = window.start.map(|start| {
        let asking = start(scope);
        let watching = asking.watch(Box::new(repaint));
        (asking, watching)
    });

    let front = Frontend {
        window: scope,
        root: window.root,
        asking,
        ui,
    };
    let outcome = eframe::run_native(
        &window.title,
        window.options,
        Box::new(|cc| {
            if let Ok(mut slot) = context.lock() {
                *slot = Some(cc.egui_ctx.clone());
            }
            Ok(Box::new(front))
        }),
    );

    if let Ok(mut slot) = context.lock() {
        *slot = None;
    }
    drop(host);
    drop(runtime);

    outcome.map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

struct Frontend {
    window: Scope,
    root: Root,
    asking: Option<(Rc<dyn Asking>, Watching)>,
    ui: UiLoop,
}

impl eframe::App for Frontend {
    /// Before the drawing, and also while the window is hidden: an actor that
    /// finished work still gets its turn on this thread.
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui.drain();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let question = self.asking.as_ref().and_then(|(asking, _)| asking.pending());

        // `add_enabled_ui(false, ..)` is the obligation the adapter contract
        // puts on every backend that draws over its own frame: while a guard's
        // question is up, the tree underneath must not take input, or the tabs
        // keep switching behind the dialog.
        ui.add_enabled_ui(question.is_none(), |ui| frame(self.window, &self.root, ui));

        if let (Some(ask), Some((asking, _))) = (question, &self.asking) {
            ask_in(ui, &ask, asking.as_ref());
        }
    }
}

fn ask_in(ui: &mut egui::Ui, ask: &guinea_core::guard::Ask, asking: &dyn Asking) {
    egui::Window::new("?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(&ask.text);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(&ask.cancel).clicked() {
                    asking.answer(false);
                }
                if ui.button(&ask.confirm).clicked() {
                    asking.answer(true);
                }
            });
        });
}
