//! Opening windows, and taking the application apart once the last one closes.

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use guinea_app::app::{GuineaApp, Stop, install_runtime, shutdown_current};
use guinea_app::feature::ScopeContext;
use guinea_router::router::{RouteChain, Router};
use windows_reactor::{
    AppProxy, Border, Component, ComponentContext, View, ViewContext, WindowVisuals,
};

use crate::winui::{NewWindow, Rooted, RouterRoot, Shown, WinUi};

/// The label this backend gives its first window, matching the other four.
pub const MAIN: &str = "main";

const E_FAIL: windows_core::HRESULT = windows_core::HRESULT(0x8000_4005_u32 as _);

thread_local! {
    static PROXY: RefCell<Option<AppProxy>> = const { RefCell::new(None) };
    static STANDING: Cell<usize> = const { Cell::new(0) };
    static FAILED: RefCell<Option<anyhow::Error>> = const { RefCell::new(None) };
    static ROUTERS: RefCell<Vec<Weak<Router<WinUi>>>> = const { RefCell::new(Vec::new()) };
}

/// A window's router, to be taken down before the application is: its
/// segments read from what the application holds.
pub(crate) fn standing(router: &Rc<Router<WinUi>>) {
    ROUTERS.with(|routers| {
        let mut routers = routers.borrow_mut();
        routers.retain(|router| router.strong_count() > 0);
        routers.push(Rc::downgrade(router));
    });
}

/// The main window's first route did not install: the application ends, and
/// [`run`] returns `error`.
pub(crate) fn failed(error: anyhow::Error) {
    FAILED.with(|failed| *failed.borrow_mut() = Some(error));

    if let Some(proxy) = PROXY.with(|slot| slot.borrow().clone())
        && let Err(error) = proxy.exit()
    {
        tracing::warn!(%error, "asking the application to exit");
    }
}

/// A second window, showing the same route tree from `initial`.
///
/// It gets its own router; the application is the one it is opened from.
pub fn window<R>(window: Window, initial: R) -> NewWindow
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    NewWindow::under(move |app| opening(app, window, initial))
}

fn opening<R>(app: &ScopeContext, window: Window, initial: R) -> View
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    let rooted = Rooted {
        app: app.clone(),
        initial,
    };
    View::component::<Root<R>>(Opening { window, rooted })
}

/// Installs `app`, opens a window at `initial`, and runs until the last
/// window closes.
///
/// `initial` runs after the plugins are installed, and is handed the
/// application's context to ask them through.
pub fn run<R>(
    app: GuineaApp,
    window: Window,
    initial: impl FnOnce(&ScopeContext) -> R + 'static,
) -> anyhow::Result<()>
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    let failure = Rc::new(RefCell::new(None));
    let startup_failure = failure.clone();

    let result = windows_reactor::App::run_with(move |cx| {
        let proxy = cx.proxy();
        crate::dispatching::install(proxy.clone());
        PROXY.with(|slot| *slot.borrow_mut() = Some(proxy));

        let runtime = match app.install() {
            Ok(runtime) => runtime,
            Err(error) => {
                *startup_failure.borrow_mut() = Some(error);
                return Err(windows_core::Error::new(E_FAIL, "installing the application"));
            }
        };
        let context = runtime.context();
        install_runtime(runtime);
        let installed = Installed;

        cx.open_component_window::<Shown>(opening(&context, window, initial(&context)))?;
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
        return Err(error.context("guinea: installing the first route"));
    }
    result.map_err(|error| anyhow::anyhow!("windows-reactor: {error}"))
}

/// Tears the application down when the reactor lets go of it - the windows'
/// route trees first, which the reactor would otherwise take down after it.
struct Installed;

impl Drop for Installed {
    fn drop(&mut self) {
        let routers = ROUTERS.with(|routers| std::mem::take(&mut *routers.borrow_mut()));
        for router in routers.iter().filter_map(Weak::upgrade) {
            router.deactivate();
        }

        shutdown_current();
    }
}

/// How the window looks, declared by the application and applied by the root.
#[derive(Clone, PartialEq)]
pub struct Window {
    title: String,
    size: Option<(f64, f64)>,
}

impl Window {
    pub fn new() -> Self {
        Self {
            title: String::new(),
            size: None,
        }
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

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, PartialEq)]
struct Opening<R> {
    window: Window,
    rooted: Rooted<R>,
}

/// The window's root: the chrome, and the route tree.
struct Root<R>(PhantomData<R>);

impl<R> Component for Root<R>
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    type Input = Opening<R>;
    type Message = ();

    fn create(_input: &Opening<R>, _cx: &ComponentContext<Self>) -> Self {
        STANDING.with(|standing| standing.set(standing.get() + 1));
        Self(PhantomData)
    }

    fn view(&self, input: &Opening<R>, cx: &mut ViewContext<Self>) -> View {
        cx.window_title(input.window.title.clone());
        if let Some((width, height)) = input.window.size {
            cx.window_visuals(WindowVisuals::new().client_size(width, height));
        }

        Border::new()
            .content(View::component::<RouterRoot<R>>(input.rooted.clone()))
            .into()
    }

    fn update(&mut self, _message: (), _cx: &ComponentContext<Self>) {}
}

impl<R> Drop for Root<R> {
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
