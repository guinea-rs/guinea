//! Opening windows, and taking the application apart once the last one closes.

use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use guinea_app::app::roots;
use guinea_app::app::{AppContext, FeatureBuilder, GuineaHost, Stop, start};
use guinea_core::scope::Scope;
use guinea_router::mount::Pass;
use guinea_router::navigator::{Asking, Navigator, Routable, Watching};
use windows_reactor::{
    AppProxy, Component, ComponentContext, ContentDialog, ContentDialogExt,
    ContentDialogResult, Grid, TextBlock, View, ViewContext, WindowVisuals,
};

use crate::winui::{Mountable, NewWindow, Shown};

/// The label this backend gives its first window, matching the other four.
pub const MAIN: &str = "main";

const E_FAIL: windows_core::HRESULT = windows_core::HRESULT(0x8000_4005_u32 as _);

thread_local! {
    static PROXY: RefCell<Option<AppProxy>> = const { RefCell::new(None) };
    static STANDING: Cell<usize> = const { Cell::new(0) };
    static FAILED: RefCell<Option<anyhow::Error>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<Vec<Scope>> = const { RefCell::new(Vec::new()) };
}

/// A window's scope, to be removed before the application is: what it mounted
/// reads from what the application holds. Whether it is the only window
/// standing - the main one.
fn standing(window: Scope) -> bool {
    WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.retain(|window| window.is_alive());
        windows.push(window);
        windows.len() == 1
    })
}

/// The main window's root did not mount: the application ends, and [`run`]
/// returns `error`.
fn failed(error: anyhow::Error) {
    FAILED.with(|failed| *failed.borrow_mut() = Some(error));

    if let Some(proxy) = PROXY.with(|slot| slot.borrow().clone())
        && let Err(error) = proxy.exit()
    {
        tracing::warn!(%error, "asking the application to exit");
    }
}

/// The application a window belongs to, kept in the window's scope.
struct Application(AppContext);

/// The application the window `from` is in belongs to.
pub(crate) fn application(from: Scope) -> Option<AppContext> {
    std::iter::successors(Some(from), Scope::parent)
        .find_map(|scope| scope.try_kept::<Application, _>(|app| app.0.clone()))
}

/// Another window of the application.
///
/// It gets a navigator of its own, when it starts somewhere; the application
/// is the one it is opened from.
pub fn window(window: Window) -> NewWindow {
    NewWindow::under(move |app| opening(app, window))
}

fn opening(app: &AppContext, window: Window) -> View {
    let rooted = Rooted {
        app: app.clone(),
        window,
    };
    View::component::<Chrome>(rooted)
}

/// Builds the application with `app` once XAML is up, opens the window it
/// returns, and runs until the last window closes.
///
/// ```no_run
/// # use guinea_app::app::FeatureBuilder;
/// # use guinea_macros::{feature, installs};
/// # use guinea_winui::reactor::{TextBlock, View};
/// # use guinea_winui::{Layout, LayoutCx, Routable, Window, layout, run};
/// # #[derive(Clone, PartialEq, Debug, Routable)]
/// # enum Route {
/// #     Processes,
/// # }
/// # #[derive(Default)]
/// # struct Shell;
/// # #[layout]
/// # impl Layout for Shell {
/// #     fn view(&self, _cx: &mut LayoutCx<'_, '_, Self>) -> View {
/// #         TextBlock::new().text("guinea").into()
/// #     }
/// # }
/// # feature! {
/// #     Activity {}
/// # }
/// # #[installs]
/// # fn activity(_app: &mut FeatureBuilder) -> anyhow::Result<Activity> {
/// #     Ok(Activity)
/// # }
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
///     app.install::<Activity>()?;
///     Ok(Window::of::<Shell>().title("guinea").at(Route::Processes))
/// }
///
/// # fn main() -> anyhow::Result<()> {
/// run(app)
/// # }
/// ```
pub fn run(app: impl FnOnce(&mut FeatureBuilder) -> anyhow::Result<Window> + 'static) -> anyhow::Result<()> {
    let failure = Rc::new(RefCell::new(None));
    let startup_failure = failure.clone();

    let result = windows_reactor::App::run_with(move |cx| {
        let proxy = cx.proxy();
        crate::dispatching::install(proxy.clone());
        PROXY.with(|slot| *slot.borrow_mut() = Some(proxy));

        let (host, window) = match start("winui", app) {
            Ok(started) => started,
            Err(error) => {
                *startup_failure.borrow_mut() = Some(error);
                return Err(windows_core::Error::new(E_FAIL, "installing the application"));
            }
        };
        let context = host.context();
        let installed = Installed { _host: host };

        cx.open_component_window::<Shown>(opening(&context, window))?;
        Ok(installed)
    });

    PROXY.with(|slot| slot.borrow_mut().take());

    if let Some(error) = failure.borrow_mut().take() {
        if error.is::<Stop>() {
            return Ok(());
        }
        return Err(error.context("guinea: installing the application"));
    }
    if let Some(error) = FAILED.with(|failed| failed.borrow_mut().take()) {
        return Err(error.context("guinea: mounting the main window"));
    }
    result.map_err(|error| anyhow::anyhow!("windows-reactor: {error}"))
}

/// The application, torn down when the reactor lets go of it - what the
/// windows mounted first, which the reactor would otherwise take down after
/// it.
struct Installed {
    _host: GuineaHost,
}

impl Drop for Installed {
    fn drop(&mut self) {
        let windows = WINDOWS.with(|windows| std::mem::take(&mut *windows.borrow_mut()));
        for window in windows.into_iter().filter(|window| window.is_alive()) {
            window.remove();
        }
    }
}

/// A window: the layout it roots at, where it starts, and how it looks.
///
/// ```
/// # use guinea_winui::reactor::{TextBlock, View};
/// # use guinea_winui::{Layout, LayoutCx, Routable, Window, layout};
/// # #[derive(Clone, PartialEq, Debug, Routable)]
/// # enum Route {
/// #     Processes,
/// # }
/// # #[derive(Default)]
/// # struct Shell;
/// # #[layout]
/// # impl Layout for Shell {
/// #     fn view(&self, _cx: &mut LayoutCx<'_, '_, Self>) -> View {
/// #         TextBlock::new().text("guinea").into()
/// #     }
/// # }
/// # let _window =
/// Window::of::<Shell>().title("guinea").at(Route::Processes)
/// # ;
/// ```
#[derive(Clone)]
pub struct Window {
    title: String,
    size: Option<(f64, f64)>,
    root: Root,
    start: Option<Rc<dyn Fn(Scope) -> Rc<dyn Asking>>>,
}

/// The layout a window roots at, by type.
#[derive(Clone, Copy)]
struct Root {
    kind: TypeId,
    mount: fn(&Pass) -> anyhow::Result<Scope>,
    name: &'static str,
}

impl Window {
    /// A window whose content is the layout or page `L`.
    pub fn of<L: Mountable<Params = ()>>() -> Self {
        Self {
            title: String::new(),
            size: None,
            root: Root {
                kind: TypeId::of::<L>(),
                mount: |pass| pass.mount::<L, ()>(&(), |cx| L::install(cx, &())),
                name: std::any::type_name::<L>(),
            },
            start: None,
        }
    }

    /// Starts the window at `route`, with a navigator of its own over `R`:
    /// what the layouts in it find with `cx.navigate::<R>()`, and whose
    /// guards' questions the window puts up as a dialog.
    pub fn at<R: Routable>(mut self, route: R) -> Self {
        self.start = Some(Rc::new(move |window| {
            let navigator = Navigator::open(window, route.clone());

            #[cfg(feature = "harness")]
            crate::harness::remember_window(window, &navigator);

            Rc::new(navigator) as Rc<dyn Asking>
        }));
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn client_size(mut self, width: f64, height: f64) -> Self {
        self.size = Some((width, height));
        self
    }
}

/// The same title, size and root, starting the same way.
impl PartialEq for Window {
    fn eq(&self, other: &Self) -> bool {
        let start = match (&self.start, &other.start) {
            (Some(mine), Some(theirs)) => Rc::ptr_eq(mine, theirs),
            (None, None) => true,
            _ => false,
        };
        self.title == other.title && self.size == other.size && self.root.kind == other.root.kind && start
    }
}

/// What a window is opened with: the application whose window it is, and
/// the window.
#[derive(Clone)]
pub(crate) struct Rooted {
    pub(crate) app: AppContext,
    pub(crate) window: Window,
}

/// The same application, the same window.
impl PartialEq for Rooted {
    fn eq(&self, other: &Self) -> bool {
        self.app == other.app && self.window == other.window
    }
}

/// A window's root: its chrome, its navigator, and the layout it roots at.
///
/// It owns the window's scope, under the application's, which is what makes
/// a window a root: the root opens it when the component is created and
/// closes it when the component is dropped.
pub(crate) struct Chrome {
    root: roots::Root,
    asking: Option<(Rc<dyn Asking>, Watching)>,
    _panel: guinea_core::observability::panels::PanelGuard,
}

/// A guard's question came or went, or the navigator moved.
pub(crate) struct Heard;

impl Component for Chrome {
    type Input = Rooted;
    type Message = Heard;

    fn create(input: &Rooted, cx: &ComponentContext<Self>) -> Self {
        STANDING.with(|standing| standing.set(standing.get() + 1));

        let root = roots::Root::under(&input.app);
        let window = root.scope();
        window.drawn_by(crate::winui::Reactor);
        window.keep(Application(input.app.clone()));
        if standing(window) {
            roots::set_label(window, MAIN);
        }

        let asking = input.window.start.as_ref().map(|start| {
            let navigator = start(window);
            let sender = cx.sender();
            let watching = navigator.watch(Box::new(move || {
                let _gone = !sender.send(Heard);
            }));
            (navigator, watching)
        });

        Self {
            _panel: crate::devtools::offer(root.id().get(), window),
            root,
            asking,
        }
    }

    fn update(&mut self, _heard: Heard, _cx: &ComponentContext<Self>) {}

    fn view(&self, input: &Rooted, cx: &mut ViewContext<Self>) -> View {
        let _caused = crate::winui::caused(cx);
        let window = &input.window;

        cx.window_title(window.title.clone());
        if let Some((width, height)) = window.size {
            cx.window_visuals(WindowVisuals::new().client_size(width, height));
        }

        let scope = self.root.scope();
        let pass = Pass::begin(scope);
        let mounted = (window.root.mount)(&pass);
        pass.end();

        let content: View = match mounted {
            Ok(root) => crate::winui::drawn(root),
            Err(error) => {
                let shown = format!("{error:#}");
                tracing::error!(error = %shown, root = window.root.name, "the window's root did not mount");
                if roots::label(scope).as_deref() == Some(MAIN) {
                    failed(error);
                }
                TextBlock::new().text(shown).into()
            }
        };

        let grid = Grid::new().children((content,));
        match &self.asking {
            Some((navigator, _)) => grid.content_dialog(question(navigator)),
            None => grid.into(),
        }
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        let last = STANDING.with(|standing| {
            let left = standing.get().saturating_sub(1);
            standing.set(left);
            left == 0
        });

        if last && let Some(proxy) = PROXY.with(|slot| slot.borrow().clone()) {
            if let Err(error) = proxy.exit() {
                tracing::warn!(%error, "asking the application to exit");
            }
        }
    }
}

/// The question a guard is waiting on, as a dialog whose buttons answer it.
///
/// Always attached to the root and shown only while there is a question, so
/// what the window mounted keeps its place whether one is asked or not.
fn question(navigator: &Rc<dyn Asking>) -> ContentDialog {
    let pending = navigator.pending();
    let open = pending.is_some();
    let ask = pending.unwrap_or_else(|| guinea_core::guard::Ask::new("", "", ""));

    let answering = navigator.clone();
    ContentDialog::new()
        .is_open(open)
        .primary_button_text(ask.confirm)
        .close_button_text(ask.cancel)
        .on_closed(move |result: ContentDialogResult| {
            answering.answer(result == ContentDialogResult::Primary);
        })
        .content(TextBlock::new().text(ask.text))
}
