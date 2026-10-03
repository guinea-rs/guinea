//! Owning the terminal: install the application, draw, read input, repeat.
//!
//! Unlike the WinUI backend, which borrows the reactor's loop, here the loop
//! is ours. That makes this the place where everything meets: the frame, the
//! router, the tasks actors queued from other threads, and the keys.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use guinea_app::app::{GuineaApp, Stop, install_runtime, shutdown_current};
use guinea_app::feature::ScopeContext;
use guinea_core::actor::UiThreadToken;
use guinea_router::router::{NavigateHandle, RouteChain, RouteSink, Router};
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::{Tui, dialog, dispatcher};

/// What the application wants after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// How often the loop wakes with nothing to do. Sets the worst-case delay
/// before work an actor finished on another thread reaches the screen.
const TICK: Duration = Duration::from_millis(50);

/// Takes over the terminal and runs until `on_event` says to stop.
///
/// `on_event` is handed every input event, the navigator, and the router, so
/// routing stays the application's decision - the same split as the reactor
/// backend, where keys are the application's business and the router only
/// obeys. The router comes along because a terminal has no widgets to hang
/// handlers on: a key is the only way to reach a page's actions, and they are
/// found through the scope the router installed.
/// `initial` is a closure rather than a value because where an application
/// starts is often something only the installed plugins know - a route saved
/// by the last run, read out of the store the store plugin just provided.
/// Called once, after `install`, before the first frame, with the
/// application's context to ask them through.
pub fn run<R, F>(
    app: GuineaApp,
    initial: impl FnOnce(&ScopeContext) -> R,
    mut on_event: F,
) -> anyhow::Result<()>
where
    R: RouteChain<Tui> + Clone + PartialEq + 'static,
    F: FnMut(&Event, &NavigateHandle<Tui, R>, &Router<Tui>) -> Flow,
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

    let router = Rc::new(Router::<Tui>::new(token));
    let route = Rc::new(RefCell::new(initial.clone()));
    let nav = NavigateHandle::new(router.clone(), {
        let route = route.clone();
        RouteSink::new(move |next: R| *route.borrow_mut() = next)
    });

    router.navigate(initial.clone())?;

    let outcome = Screen::enter().and_then(|mut screen| {
        let outcome = pump(&mut screen.terminal, &router, &nav, &mut on_event);
        let left = screen.leave();
        outcome.and(left)
    });

    router.deactivate();
    shutdown_current();
    outcome
}

fn pump<R, F>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    router: &Router<Tui>,
    nav: &NavigateHandle<Tui, R>,
    on_event: &mut F,
) -> anyhow::Result<()>
where
    R: RouteChain<Tui> + Clone + PartialEq + 'static,
    F: FnMut(&Event, &NavigateHandle<Tui, R>, &Router<Tui>) -> Flow,
{
    loop {
        let asking = router.pending();

        terminal.draw(|frame| {
            router.render(&()).draw(frame, frame.area());
            if let Some(ask) = &asking {
                dialog::draw(frame, ask);
            }
        })?;

        if event::poll(TICK)? {
            let event = event::read()?;

            // While a question is up the application does not see the keys.
            // Without this the tabs keep switching underneath the dialog -
            // the obligation every backend that draws over its own frame
            // carries, and the easiest one to forget.
            match &asking {
                Some(_) => dialog::answer(router, &event),
                None => {
                    if on_event(&event, nav, router) == Flow::Exit {
                        return Ok(());
                    }
                }
            }
        }

        // After the keys, so a navigation made above is already installed and
        // whatever it started can run in the same breath.
        dispatcher::drain();
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
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}
