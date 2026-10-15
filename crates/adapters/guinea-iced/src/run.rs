//! Installing the application into the loop iced owns.
//!
//! The application is installed before the window opens, as in every other
//! backend: `install()` runs on this thread, actors start, and only then is the
//! loop handed over. What iced adds is that the loop is reached through three
//! free functions rather than a trait object, so what the window holds travels
//! in the state iced keeps for us.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::app::{AppRuntime, GuineaApp, Stop};
use guinea_app::feature::FeatureHost;
use guinea_core::guard::Ask;
use guinea_core::scope::Scope;
use guinea_core::ui_thread::UiLoop;
use guinea_router::mount::Pass;
use guinea_router::navigator::{Asking, Navigator, Routable};
use iced::widget::{button, center, column, container, opaque, row, text};

use crate::{Element, Envelope, Mountable, Nodes, drawn, envelope, failed, wakeups};

/// What [`run`] calls the window it opens.
pub const MAIN: &str = "main";

/// A window: the layout it roots at, where it starts, and how it looks.
///
/// ```ignore
/// Window::of::<Shell>().title("guinea").at(Route::Processes)
/// ```
pub struct Window {
    title: String,
    settings: iced::window::Settings,
    root: Root,
    start: Option<Box<dyn FnOnce(Scope) -> Rc<dyn Asking>>>,
}

/// The layout a window roots at.
pub(crate) struct Root {
    mount: for<'a> fn(&Pass, &'a Nodes) -> anyhow::Result<Scope>,
}

impl Root {
    pub(crate) fn of<L: Mountable<Params = ()>>() -> Self {
        Self {
            mount: |pass, nodes| {
                let root = pass.mount::<L, ()>(&(), |cx| L::install(cx, &()))?;
                nodes.adopt(root);
                Ok(root)
            },
        }
    }
}

impl Window {
    /// A window whose content is the layout or page `L`.
    pub fn of<L: Mountable<Params = ()>>() -> Self {
        Self {
            title: String::new(),
            settings: iced::window::Settings::default(),
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

    /// How iced opens the window: its size, position, decorations.
    pub fn settings(mut self, settings: iced::window::Settings) -> Self {
        self.settings = settings;
        self
    }
}

/// One frame of the window whose scope is `window`: its root, mounted, and
/// what that mounts in turn.
pub(crate) fn frame<'a>(window: Scope, root: &Root, nodes: &'a Nodes) -> Element<'a, Envelope> {
    let pass = Pass::begin(window);
    let mounted = (root.mount)(&pass, nodes);
    pass.end();

    match mounted {
        Ok(root) => drawn(root, nodes),
        Err(error) => failed(&error),
    }
}

/// The state iced keeps for us: the window's scope, what navigates it, and
/// the nodes of what it mounted.
///
/// The reducers live in each node's scope. The nodes cannot: a view borrows
/// its node for the frame, so they live here - see [`Nodes`].
pub struct Shell {
    window: Scope,
    root: Root,
    asking: Option<Rc<dyn Asking>>,
    title: String,
    nodes: Nodes,
    /// Where work from other threads waits for the next turn.
    ui: UiLoop,
}

/// Installs `app`, opens the window `main` makes, and runs until it closes.
///
/// `main` runs after the plugins are installed, and is handed the installed
/// application to read what they provided from - where to start, say.
///
/// ```ignore
/// guinea_iced::run(app, |_| Window::of::<Shell>().title("guinea").at(Route::Processes))
/// ```
///
/// A root that fails to mount ends it with why, before the window opens.
pub fn run(app: GuineaApp, main: impl FnOnce(&AppRuntime) -> Window) -> anyhow::Result<()> {
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread.
    let ui = wakeups::install();

    let runtime = match app.start() {
        Ok(runtime) => runtime,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };
    let window = main(&runtime);

    let host = FeatureHost::under(&runtime.context());
    let scope = host.scope();
    guinea_app::app::roots::set_label(scope, MAIN);
    let asking = window.start.map(|start| start(scope));

    let nodes = Nodes::default();
    let pass = Pass::begin(scope);
    let mounted = (window.root.mount)(&pass, &nodes);
    pass.end();
    if let Err(error) = mounted {
        return Err(error.context("guinea: mounting the main window"));
    }

    let booting = RefCell::new(Some(Shell {
        window: scope,
        root: window.root,
        asking,
        title: window.title,
        nodes,
        ui,
    }));
    let boot = move || {
        // Called once by iced; a second call would find the shell gone,
        // which is a louder failure than silently starting empty.
        booting.borrow_mut().take().expect("iced boots an application once")
    };

    let outcome = iced::application(boot, update, view)
        .title(|shell: &Shell| shell.title.clone())
        .subscription(subscription)
        .window(window.settings)
        .run();

    wakeups::close();
    drop(host);
    drop(runtime);

    outcome.map_err(|e| anyhow::anyhow!("iced: {e}"))
}

/// Every message is a turn, and a turn begins with what other threads handed
/// over: running it is what makes observers fire, and the settling after the
/// message picks up whatever they parked.
fn update(shell: &mut Shell, message: Envelope) {
    shell.ui.drain();
    envelope::settle(&mut shell.nodes, shell.asking.as_deref(), message);
}

fn view(shell: &Shell) -> Element<'_, Envelope> {
    let tree = frame(shell.window, &shell.root, &shell.nodes);

    // A guard's question is the navigator's, so it is drawn like anything
    // else - and `opaque` is the obligation the adapter contract puts on
    // every backend that draws over its own frame: while a question is up,
    // the tree underneath must not take input, or the tabs keep switching
    // behind the dialog.
    match shell.asking.as_ref().and_then(|asking| asking.pending()) {
        None => tree,
        Some(ask) => iced::widget::stack![tree, opaque(question(ask))].into(),
    }
}

/// The dialog's own message.
///
/// An [`Envelope`] carries a node's message erased, so it cannot be cloned -
/// and iced's `button` asks for exactly that. So the shell's own chrome is a
/// node like any other: it speaks its own message and is sealed on the way
/// out, the same seam a layout draws with `cx.mine`.
#[derive(Clone, Copy)]
enum Answer {
    Yes,
    No,
}

fn question(ask: Ask) -> Element<'static, Envelope> {
    let buttons = row![
        button(text(ask.cancel)).on_press(Answer::No),
        button(text(ask.confirm))
            .style(button::danger)
            .on_press(Answer::Yes),
    ]
    .spacing(8);

    let dialog = center(
        container(column![text(ask.text), buttons].spacing(16))
            .padding(20)
            .style(container::bordered_box),
    )
    .style(|theme: &iced::Theme| container::Style {
        background: Some(
            theme
                .extended_palette()
                .background
                .base
                .color
                .scale_alpha(0.8)
                .into(),
        ),
        ..container::Style::default()
    });

    Element::from(dialog).map(|answer| Envelope::answer(matches!(answer, Answer::Yes)))
}

/// The only subscription: work finished on another thread asking for a turn.
fn subscription(_shell: &Shell) -> iced::Subscription<Envelope> {
    iced::Subscription::run(wakeups::wakeups)
}
