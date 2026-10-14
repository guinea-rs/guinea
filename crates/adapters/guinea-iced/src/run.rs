//! Installing the application into the loop iced owns.
//!
//! The route tree is built before the window opens, as in every other backend:
//! `install()` runs on this thread, actors start, and only then is the loop
//! handed over. What iced adds is that the loop is reached through three free
//! functions rather than a trait object, so the router travels in the state
//! iced holds for us.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::app::{AppRuntime, GuineaApp, Stop};
use guinea_app::feature::FeatureHost;
use guinea_core::guard::Ask;
use guinea_core::ui_thread::UiLoop;
use guinea_router::router::{NavigateHandle, RouteChain, RouteSink, Router};
use iced::widget::{button, center, column, container, opaque, row, text};

use crate::{Envelope, Iced, Nodes, envelope, nav, wakeups};

/// What [`run`] calls the root it opens.
pub const MAIN: &str = "main";

/// The state iced keeps for us: the router, and the nodes of the chain it
/// mounted.
///
/// The reducers live in each segment's scope, where the router's cache
/// carries them across navigation. The nodes cannot: a view borrows its node
/// for the render, so they live here, and [`Nodes`] keeps the ones a page
/// asked to keep by the router's rules.
pub struct Shell {
    router: Rc<Router<Iced>>,
    /// The nodes of the mounted chain, owned here because a view borrows them
    /// - see [`Nodes`]. This is the one thing iced hands out by reference for
    /// exactly the render, which is what a borrowing widget needs.
    nodes: Nodes,
    /// Where work from other threads waits for the next turn.
    ui: UiLoop,
}

/// Runs the application in a window iced opens, starting where `initial` says.
///
/// ```ignore
/// guinea_iced::run(app, "Processes", iced::window::Settings::default(), initial_route)
/// ```
///
/// A closure rather than a value because where an application starts is often
/// something only the installed plugins know - a route saved by the last run,
/// read out of the store the store plugin just provided. Called once, after
/// `install`, before the first frame, with the installed application to read
/// them from.
pub fn run<R>(
    app: GuineaApp,
    title: &str,
    window: iced::window::Settings,
    initial: impl FnOnce(&AppRuntime) -> R,
) -> anyhow::Result<()>
where
    R: RouteChain<Iced> + Clone + PartialEq + 'static,
{
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread.
    let ui = wakeups::install();

    // Genuinely this thread: it is the one that will draw, and nothing else
    // touches the router or the scopes.
    let runtime = match app.install() {
        Ok(runtime) => runtime,
        Err(error) if error.is::<Stop>() => return Ok(()),
        Err(error) => return Err(error),
    };
    let context = runtime.context();
    let initial = initial(&runtime);

    let router = Rc::new(Router::<Iced>::new(FeatureHost::under(&context)));
    guinea_app::app::roots::set_label(router.host().scope(), MAIN);

    let route = Rc::new(RefCell::new(initial.clone()));
    nav::install(NavigateHandle::new(router.clone(), {
        let route = route.clone();
        RouteSink::new(move |next: R| *route.borrow_mut() = next)
    }));

    router.navigate(initial)?;

    // The first chain is installed before iced exists, so its nodes are taken
    // out of staging here rather than in the first update.
    let mut nodes = Nodes::default();
    if let Some(chain) = router.active_chain() {
        nodes.sync(chain);
    }
    let booting = RefCell::new(Some((nodes, ui)));

    let title = title.to_string();
    let boot = {
        let router = router.clone();
        move || {
            // Called once by iced; a second call would find the store gone,
            // which is a louder failure than silently starting empty.
            let (nodes, ui) = booting
                .borrow_mut()
                .take()
                .expect("iced boots an application once");

            Shell {
                router: router.clone(),
                nodes,
                ui,
            }
        }
    };

    let outcome = iced::application(boot, update, view)
        .title(move |_: &Shell| title.clone())
        .subscription(subscription)
        .window(window)
        .run();

    wakeups::close();
    nav::clear();
    router.deactivate();
    drop(runtime);

    outcome.map_err(|e| anyhow::anyhow!("iced: {e}"))
}

/// Every message is a turn, and a turn begins with what other threads handed
/// over: running it is what makes observers fire, and the settling after the
/// message picks up whatever they parked.
fn update(shell: &mut Shell, message: Envelope) {
    shell.ui.drain();
    envelope::settle(&shell.router, &mut shell.nodes, message);
}

fn view(shell: &Shell) -> iced::Element<'_, Envelope> {
    let tree = shell.router.render(&shell.nodes);

    // A guard's question is router state, so it is drawn like anything else -
    // and `opaque` is the obligation the adapter contract puts on every
    // backend that draws over its own frame: while a question is up, the tree
    // underneath must not take input, or the tabs keep switching behind the
    // dialog.
    match shell.router.pending() {
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

fn question(ask: Ask) -> iced::Element<'static, Envelope> {
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

    iced::Element::from(dialog).map(|answer| Envelope::answer(matches!(answer, Answer::Yes)))
}

/// The only subscription: work finished on another thread asking for a turn.
fn subscription(_shell: &Shell) -> iced::Subscription<Envelope> {
    iced::Subscription::run(wakeups::wakeups)
}
