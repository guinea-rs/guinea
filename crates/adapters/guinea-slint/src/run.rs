//! Installing the application into a window Slint owns.
//!
//! The loop belongs to Slint, the way it belongs to the reactor under WinUI
//! and to us under ratatui. What is left here is the wiring: the dispatcher,
//! the host, the window's navigator and its global, and mounting again
//! whenever the navigator moves.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use guinea_app::app::roots::{self, RootId};
use guinea_app::app::windows::{SavedGeometry, WindowService, Windows};
use guinea_app::app::{FeatureBuilder, Stop, start};
use guinea_core::scope::Scope;
use guinea_router::mount::Pass;
use guinea_router::navigator::{Navigator, Watching};
use slint::ComponentHandle;

use crate::windows::SlintWindows;
use crate::{Component, Mountable, Shows, descend, dispatcher, windows};

guinea_macros::plugin! {
    BackendWindows {
        provides { WindowService }
    }
}

#[guinea_macros::builds]
fn backend_windows(
    _app: &mut guinea_app::app::PluginBuilder,
    windows: WindowService,
) -> anyhow::Result<BackendWindows> {
    Ok(BackendWindows(windows))
}

/// What [`run`] calls the root it opens.
pub const MAIN: &str = "main";

/// A window: the component Slint draws, the layout that mounts under it, and
/// where it starts.
///
/// ```
/// # use guinea_slint::{Layout, LayoutCx, Navigator, Routable, Shows, Window, layout};
/// # slint::slint! {
/// #     export component AppWindow inherits Window {}
/// # }
/// # #[derive(Clone, PartialEq, Debug, Routable)]
/// # enum Route {
/// #     Processes,
/// # }
/// # impl Shows<AppWindow> for Route {
/// #     fn show(_window: &AppWindow, _nav: &Navigator<Self>) {}
/// #     fn listen(_window: &AppWindow, _nav: &Navigator<Self>) {}
/// # }
/// # #[derive(Default)]
/// # struct Shell;
/// # #[layout]
/// # impl Layout for Shell {
/// #     fn mount(&mut self, _cx: &mut LayoutCx<'_, Self>) {}
/// # }
/// # fn main() -> anyhow::Result<()> {
/// # i_slint_backend_testing::init_no_event_loop();
/// # let _window =
/// Window::of::<Shell>(AppWindow::new()?).at(Route::Processes)
/// # ;
/// # Ok(())
/// # }
/// ```
pub struct Window<W> {
    component: W,
    root: Root,
    start: Option<Box<Start<W>>>,
    remembered: Option<SavedGeometry>,
}

type Start<W> = dyn FnOnce(Scope, &W, Remount) -> Started;

/// Mounts the window again, and says why the root did not mount, if it did
/// not.
type Remount = Rc<dyn Fn() -> anyhow::Result<()>>;

struct Started {
    show: Rc<dyn Fn()>,
    watching: Watching,
}

/// What an open window holds on to: dropped, the window stops following its
/// navigator.
#[must_use = "the window follows its navigator only while this is held"]
pub struct Opened {
    _watching: Option<Watching>,
}

/// The layout a window roots at.
pub(crate) struct Root {
    mount: fn(&Pass) -> anyhow::Result<Scope>,
}

impl Root {
    fn of<L: Mountable<Params = ()>>() -> Self {
        Self {
            mount: |pass| pass.mount::<L, ()>(&(), |cx| L::install(cx, &())),
        }
    }
}

impl<W: ComponentHandle + 'static> Window<W> {
    /// A window drawn by `component`, whose content the layout or page `L`
    /// mounts.
    pub fn of<L: Mountable<Params = ()>>(component: W) -> Self {
        Self {
            component,
            root: Root::of::<L>(),
            start: None,
            remembered: None,
        }
    }

    /// Opens the window where `saved` remembers it, before it shows: what a
    /// plugin remembering windows hands the application.
    ///
    /// ```
    /// # use guinea_app::app::windows::{Geometry, RestoreGeometry, SavedGeometry};
    /// # use guinea_app::app::{FeatureBuilder, PluginBuilder};
    /// # use guinea_macros::{builds, plugin};
    /// # use guinea_slint::{Layout, LayoutCx, Window, layout};
    /// # slint::slint! {
    /// #     export component AppWindow inherits Window {}
    /// # }
    /// # #[derive(Default)]
    /// # struct Shell;
    /// # #[layout]
    /// # impl Layout for Shell {
    /// #     fn mount(&mut self, _cx: &mut LayoutCx<'_, Self>) {}
    /// # }
    /// # struct Remembered;
    /// # impl RestoreGeometry for Remembered {
    /// #     fn for_label(&self, _label: &str) -> Option<Geometry> {
    /// #         None
    /// #     }
    /// # }
    /// # plugin! {
    /// #     WindowStatePlugin {
    /// #         provides { SavedGeometry }
    /// #     }
    /// # }
    /// # #[builds]
    /// # fn window_state_plugin(_app: &mut PluginBuilder, _store: &'static str) -> anyhow::Result<WindowStatePlugin> {
    /// #     Ok(WindowStatePlugin(SavedGeometry::new(Remembered)))
    /// # }
    /// # fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window<AppWindow>> {
    /// # let internal = "state.db";
    /// let WindowStatePlugin(saved) = app.plugin::<WindowStatePlugin>(internal)?;
    /// Ok(Window::of::<Shell>(AppWindow::new()?).remembered_by(saved))
    /// # }
    /// # fn main() -> anyhow::Result<()> {
    /// # i_slint_backend_testing::init_no_event_loop();
    /// # let (_host, _window) = guinea_app::app::start("doc", app)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn remembered_by(mut self, saved: SavedGeometry) -> Self {
        self.remembered = Some(saved);
        self
    }

    /// Starts the window at `route`, with a navigator of its own over `R`:
    /// what the layouts in it find with `cx.navigate::<R>()`, and what the
    /// component's `Navigator` global shows and moves.
    pub fn at<R: Shows<W>>(mut self, route: R) -> Self {
        self.start = Some(Box::new(move |window, component: &W, remount: Remount| {
            let nav = Navigator::open(window, route);
            R::listen(component, &nav);

            let shown = component.as_weak();
            let heard = nav.clone();
            let show: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(component) = shown.upgrade() {
                    R::show(&component, &heard);
                }
            });

            let again = show.clone();
            let watching = nav.watch(move || {
                if let Err(error) = remount() {
                    tracing::error!(error = format!("{error:#}"), "the window's root did not mount");
                }
                again();
            });

            Started { show, watching }
        }));
        self
    }

    /// The component Slint draws.
    pub fn component(&self) -> &W {
        &self.component
    }

    /// Opens the window in `window`: mounts its root and what is under it,
    /// and has the navigator mount again every time it moves.
    ///
    /// What [`run`] does once the application is installed, without the
    /// loop: for tests, and for an application that runs Slint's loop
    /// itself.
    pub fn open(self, window: Scope) -> anyhow::Result<Opened> {
        window.drawn_by(crate::Properties);
        window.keep(Component(Rc::new(self.component.clone_strong())));

        let remount = remounter(window, self.root);
        let started = self
            .start
            .map(|start| start(window, &self.component, remount.clone()));

        remount()?;
        if let Some(started) = &started {
            (started.show)();
        }

        Ok(Opened {
            _watching: started.map(|started| started.watching),
        })
    }
}

/// One mounting of the window whose scope is `window`.
fn pass(window: Scope, root: &Root) -> anyhow::Result<()> {
    let pass = Pass::begin(window);
    let mounted = (root.mount)(&pass);
    pass.end();

    let root = mounted?;
    descend(root);
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Passing {
    Idle,
    Running,
    Again,
}

/// Mounts the window, once at a time: a move made while it mounts - a page
/// that fails and puts the navigator back - mounts again after this one
/// instead of inside it.
fn remounter(window: Scope, root: Root) -> Remount {
    let passing = Rc::new(Cell::new(Passing::Idle));

    Rc::new(move || {
        if passing.get() != Passing::Idle {
            passing.set(Passing::Again);
            return Ok(());
        }

        let outcome = loop {
            passing.set(Passing::Running);
            let outcome = pass(window, &root);
            if passing.get() == Passing::Running {
                break outcome;
            }
        };

        passing.set(Passing::Idle);
        outcome
    })
}

/// Builds the application with `app`, opens the window it returns, and runs
/// until it closes.
///
/// ```no_run
/// # use guinea_app::app::FeatureBuilder;
/// # use guinea_macros::{feature, installs};
/// # use guinea_slint::{Layout, LayoutCx, Navigator, Routable, Shows, Window, layout};
/// # slint::slint! {
/// #     export component AppWindow inherits Window {}
/// # }
/// # #[derive(Clone, PartialEq, Debug, Routable)]
/// # enum Route {
/// #     Processes,
/// # }
/// # impl Shows<AppWindow> for Route {
/// #     fn show(_window: &AppWindow, _nav: &Navigator<Self>) {}
/// #     fn listen(_window: &AppWindow, _nav: &Navigator<Self>) {}
/// # }
/// # #[derive(Default)]
/// # struct Shell;
/// # #[layout]
/// # impl Layout for Shell {
/// #     fn mount(&mut self, _cx: &mut LayoutCx<'_, Self>) {}
/// # }
/// # feature! {
/// #     Activity {}
/// # }
/// # #[installs]
/// # fn activity(_app: &mut FeatureBuilder) -> anyhow::Result<Activity> {
/// #     Ok(Activity)
/// # }
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window<AppWindow>> {
///     app.install::<Activity>()?;
///     Ok(Window::of::<Shell>(AppWindow::new()?).at(Route::Processes))
/// }
///
/// # fn main() -> anyhow::Result<()> {
/// guinea_slint::run(app)
/// # }
/// ```
///
/// A root that fails to mount ends it with why, before the window shows.
pub fn run<W: ComponentHandle + 'static>(
    app: impl FnOnce(&mut FeatureBuilder) -> anyhow::Result<Window<W>>,
) -> anyhow::Result<()> {
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread.
    dispatcher::install();

    let shell = Arc::new(SlintWindows::default());
    let windows = WindowService::from_arc(shell.clone());

    let (host, window) = match start("slint", |builder| {
        builder.plugin::<BackendWindows>(windows)?;
        app(builder)
    }) {
        Ok(started) => started,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };
    let context = host.context();
    let saved = window.remembered.clone();

    let main = roots::Root::under(&context);
    let scope = main.scope();

    // Named, so that anything remembering something about this window between
    // runs has a key that outlives the id.
    roots::set_label(scope, MAIN);

    let component = window.component.clone_strong();
    let root_id = main.id();
    shell.attach(root_id, Some(MAIN.to_string()), context.__scope(), &component);
    restore(&shell, root_id, MAIN, saved);
    let _watching = windows::watch(shell.clone());

    let opened = window.open(scope);
    let outcome = match opened {
        Ok(opened) => {
            let outcome = component.run();
            drop(opened);
            outcome.map_err(anyhow::Error::from)
        }
        Err(error) => Err(error.context("guinea: mounting the main window")),
    };

    shell.detach(root_id);
    drop(main);
    drop(host);
    outcome
}

/// Puts the window back where it was last time, if anything remembers.
///
/// Before the loop starts, which is the whole point: a window restored after
/// it is on screen jumps, and a jump is what this is meant to avoid.
fn restore(shell: &SlintWindows, root: RootId, label: &str, saved: Option<SavedGeometry>) {
    let Some(saved) = saved else {
        return;
    };
    let Some(geometry) = saved.for_label(label) else {
        return;
    };

    tracing::debug!(%root, %label, ?geometry, "restoring window");
    if shell.apply(root, geometry).is_err() {
        tracing::debug!(%root, "the window would not take the saved geometry");
    }
}
