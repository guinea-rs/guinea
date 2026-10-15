//! Owning the terminal: install the application, draw, read input, repeat.
//!
//! Unlike the WinUI backend, which borrows the reactor's loop, here the loop
//! is ours. That makes this the place where everything meets: the frame, the
//! window's navigator, the tasks actors queued from other threads, and the
//! keys.

use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use guinea_app::app::{FeatureBuilder, Stop, start};
use guinea_app::feature::FeatureHost;
use guinea_core::scope::Scope;
use guinea_core::ui_thread::UiLoop;
use guinea_router::mount::Pass;
use guinea_router::navigator::{Asking, Navigator, Routable};
use ratatui::Terminal;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;

use crate::{Mountable, Shown, closed, dialog, drawn, failed, offer};

/// How often the loop wakes with nothing to do. Sets the worst-case delay
/// before work an actor finished on another thread reaches the screen.
const TICK: Duration = Duration::from_millis(50);

/// The terminal as a window: the layout it roots at, and where it starts.
///
/// ```ignore
/// Window::of::<Shell>().at(Route::Processes)
/// ```
pub struct Window {
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
}

/// Makes `window` a window: somewhere to keep what its frames drew.
pub(crate) fn open(window: Scope) {
    window.keep(Shown::default());
}

/// One frame of the window whose scope is `window`: its root, mounted, and
/// drawn into `area`.
pub(crate) fn frame(window: Scope, root: &Root, frame: &mut ratatui::Frame, area: Rect) {
    window.kept::<Shown, _>(|shown| shown.drawn.clear());

    let pass = Pass::begin(window);
    let mounted = (root.mount)(&pass);
    pass.end();

    match mounted {
        Ok(root) => drawn(window, root).draw(frame, area),
        Err(error) => failed(&error).draw(frame, area),
    }
}

/// Builds the application with `app`, takes over the terminal with the window
/// it returns, and runs until a segment [`closes`](crate::InputCx::close) it.
///
/// ```ignore
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
///     app.install::<Activity>()?;
///     Ok(Window::of::<Shell>().at(Route::Processes))
/// }
///
/// guinea_ratatui::run(app)
/// ```
///
/// A root that fails to mount ends it with why, before the terminal is taken.
pub fn run(app: impl FnOnce(&mut FeatureBuilder) -> anyhow::Result<Window>) -> anyhow::Result<()> {
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread. The loop below drains it
    // between frames, and wakes every tick anyway, so nothing has to wake it.
    let ui = UiLoop::install();

    let (runtime, window) = match start(app) {
        Ok(started) => started,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };

    let host = FeatureHost::under(&runtime.context());
    let scope = host.scope();
    guinea_app::app::roots::set_label(scope, MAIN);
    open(scope);

    let asking = window.start.map(|start| start(scope));

    let pass = Pass::begin(scope);
    let mounted = (window.root.mount)(&pass);
    pass.end();
    if let Err(error) = mounted {
        return Err(error.context("guinea: mounting the main window"));
    }

    let outcome = Screen::enter().and_then(|mut screen| {
        let outcome = pump(&mut screen, &ui, scope, &window.root, asking.as_deref());
        let left = screen.leave();
        outcome.and(left)
    });

    drop(asking);
    drop(host);
    drop(runtime);
    outcome
}

/// What [`run`] calls the window it opens.
pub const MAIN: &str = "main";

fn pump(
    screen: &mut Screen,
    ui: &UiLoop,
    window: Scope,
    root: &Root,
    asking: Option<&dyn Asking>,
) -> anyhow::Result<()> {
    loop {
        let question = asking.and_then(Asking::pending);

        screen.terminal.draw(|drawing| {
            let area = drawing.area();
            frame(window, root, drawing, area);
            if let Some(ask) = &question {
                dialog::draw(drawing, ask);
            }
        })?;

        if event::poll(TICK)? {
            let event = event::read()?;

            hear(window, asking, &event);
            if closed(window) {
                return Ok(());
            }
        }

        // After the keys, so what a key started can run in the same breath.
        ui.drain();
    }
}

/// Hands `event` to the question that is up, or else to what the window drew.
///
/// While a question is up the window does not see the keys. Without this the
/// tabs keep switching underneath the dialog - the obligation every backend
/// that draws over its own frame carries, and the easiest one to forget.
pub(crate) fn hear(window: Scope, asking: Option<&dyn Asking>, event: &Event) {
    match asking.filter(|asking| asking.pending().is_some()) {
        Some(asking) => dialog::answer(asking, event),
        None => {
            offer(window, event);
        }
    }
}

type PanicHook = dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static;

/// The terminal in raw mode on the alternate screen, for as long as this
/// lives.
///
/// Put back however the loop ends: a return, an error, or a panic - whose
/// message the hook prints only after the screen is back, since on the
/// alternate screen it would vanish with it.
struct Screen {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    /// The hook that stood before this one, to stand again once it is gone.
    previous: Option<Arc<PanicHook>>,
    left: bool,
}

impl Screen {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;

        let entered = execute!(io::stdout(), EnterAlternateScreen)
            .and_then(|()| Terminal::new(CrosstermBackend::new(io::stdout())));
        let terminal = match entered {
            Ok(terminal) => terminal,
            Err(error) => {
                restore();
                return Err(error.into());
            }
        };
        let _ = execute!(io::stdout(), EnableBracketedPaste);

        let previous: Arc<PanicHook> = Arc::from(std::panic::take_hook());
        let chained = previous.clone();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            chained(info);
        }));

        Ok(Self {
            terminal,
            previous: Some(previous),
            left: false,
        })
    }

    fn leave(mut self) -> anyhow::Result<()> {
        self.left = true;
        self.stand_down();

        let _ = execute!(self.terminal.backend_mut(), DisableBracketedPaste);
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    /// Puts the previous panic hook back - not while panicking, when the hook
    /// cannot be changed and is about to be needed.
    fn stand_down(&mut self) {
        if std::thread::panicking() {
            return;
        }
        if let Some(previous) = self.previous.take() {
            std::panic::set_hook(Box::new(move |info| previous(info)));
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if !self.left {
            self.stand_down();
            restore();
        }
    }
}

/// The terminal as it was before [`Screen::enter`], as far as it can be put
/// back: whatever fails here has no one left to report to.
fn restore() {
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}
