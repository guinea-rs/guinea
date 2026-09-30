//! Installing the application into the loop iced owns.
//!
//! The route tree is built before the window opens, as in every other backend:
//! `install()` runs on this thread, actors start, and only then is the loop
//! handed over. What iced adds is that the loop is reached through three free
//! functions rather than a trait object, so the router travels in the state
//! iced holds for us.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::app::{GuineaApp, install_runtime, shutdown_current};
use guinea_core::actor::UiThreadToken;
use guinea_core::guard::Ask;
use guinea_router::router::{NavigateHandle, RouteChain, RouteSink, Router};
use iced::widget::{button, center, column, container, opaque, row, text};

use crate::{Envelope, Iced, Nodes, dispatcher, envelope, nav};

/// What [`run`] calls the root it opens.
pub const MAIN: &str = "main";

/// The state iced keeps for us: the router, and nothing else.
///
/// Every node's own state lives in its segment's scope, where the router's
/// cache can carry it across navigation - so there is nothing here for one to
/// be kept in.
pub struct Shell {
    router: Rc<Router<Iced>>,
    /// The nodes of the mounted chain, owned here because a view borrows them
    /// - see [`Nodes`]. This is the one thing iced hands out by reference for
    /// exactly the render, which is what a borrowing widget needs.
    nodes: Nodes,
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
/// `install`, before the first frame.
pub fn run<R>(
    app: GuineaApp,
    title: &str,
    window: iced::window::Settings,
    initial: impl FnOnce() -> R,
) -> anyhow::Result<()>
where
    R: RouteChain<Iced> + Clone + PartialEq + 'static,
{
    // Before any actor exists: the first thing a feature does during install
    // may already queue work back to this thread.
    dispatcher::install();

    // Genuinely this thread: it is the one that will draw, and nothing else
    // touches the router or the scopes.
    let token = UiThreadToken::dangerously_create_token_unchecked();
    install_runtime(app.install(token.clone())?);

    let initial = initial();
    let router = Rc::new(Router::<Iced>::new(token));
    guinea_app::app::roots::set_label(router.root(), MAIN);

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
    let nodes = RefCell::new(Some(nodes));

    let title = title.to_string();
    let boot = {
        let router = router.clone();
        move || Shell {
            router: router.clone(),
            // Called once by iced; a second call would find the store gone,
            // which is a louder failure than silently starting empty.
            nodes: nodes
                .borrow_mut()
                .take()
                .expect("iced boots an application once"),
        }
    };

    let outcome = iced::application(boot, update, view)
        .title(move |_: &Shell| title.clone())
        .subscription(subscription)
        .window(window)
        .run();

    dispatcher::close();
    nav::clear();
    router.deactivate();
    shutdown_current();

    outcome.map_err(|e| anyhow::anyhow!("iced: {e}"))
}

fn update(shell: &mut Shell, message: Envelope) {
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
    iced::Subscription::run(dispatcher::wakeups)
}
