//! `routes!` targeting a backend other than the facade's.
//!
//! An integration test rather than a unit one on purpose: inside the crate the
//! macro resolves `guinea` as `Itself`, and what an application compiles is the
//! other case.

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_macros::routes;
use guinea_ratatui::{LayoutCx, PageCx, Tui};
use guinea_router::router::{RouteChain, Router};
use ratatui::layout::{Constraint, Direction, Layout as RLayout};
use ratatui::widgets::Paragraph;
use ratatui::{Terminal, backend::TestBackend};

#[derive(Default)]
struct Shell;

impl guinea_ratatui::Layout for Shell {
    type Params = ShellParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn render(&mut self, cx: &mut LayoutCx<'_, '_, Self>) {
        let chunks = RLayout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(cx.area());

        cx.frame().render_widget(Paragraph::new("tabs"), chunks[0]);
        cx.outlet(chunks[1]);
    }
}

#[derive(Default)]
struct Processes;

impl guinea_ratatui::Page for Processes {
    type Params = ProcessesParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ProcessesParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let area = cx.area();
        cx.frame().render_widget(Paragraph::new("processes"), area);
    }
}

#[derive(Default)]
struct Services;

impl guinea_ratatui::Page for Services {
    type Params = ServicesParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ServicesParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let area = cx.area();
        cx.frame().render_widget(Paragraph::new("services"), area);
    }
}

routes! {
    backend = guinea_ratatui::Tui,
    Route {
        layout(Shell) {
            page(Processes) link("/:host/processes") { host: String }
            page(Services) link("/:host/services") { host: String }
        }
    }
}

fn draw(route: Route) -> String {
    let router = std::rc::Rc::new(Router::<Tui>::new(FeatureHost::detached()));
    router.navigate(route).expect("navigate");

    let mut terminal = Terminal::new(TestBackend::new(10, 2)).expect("terminal");
    terminal
        .draw(|frame| router.render(&()).draw(frame, frame.area()))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_macro_builds_chains_for_a_backend_that_is_not_the_facade() {
    assert_eq!(
        draw(Route::Processes {
            host: "ubuntu".to_string()
        }),
        "tabs\nprocesses"
    );
}

#[test]
fn navigating_between_siblings_keeps_the_layout_and_swaps_the_page() {
    assert_eq!(
        draw(Route::Services {
            host: "ubuntu".to_string()
        }),
        "tabs\nservices"
    );
}

#[test]
fn a_route_still_renders_the_address_it_agreed_to() {
    let route = Route::Services {
        host: "ubuntu".to_string(),
    };
    assert_eq!(route.link().as_deref(), Some("/ubuntu/services"));
    assert_eq!(route.chain().len(), 2, "layout plus page");
}

/// A navigator over a route cell, the way `run` builds one.
fn navigator() -> (
    std::rc::Rc<Router<Tui>>,
    guinea_router::router::NavigateHandle<Tui, Route>,
    std::rc::Rc<std::cell::RefCell<Route>>,
) {
    use guinea_router::router::{NavigateHandle, RouteSink};

    let router = std::rc::Rc::new(Router::<Tui>::new(FeatureHost::detached()));
    let here = Route::Processes {
        host: "ubuntu".to_string(),
    };
    let cell = std::rc::Rc::new(std::cell::RefCell::new(here.clone()));
    let nav = NavigateHandle::new(router.clone(), {
        let cell = cell.clone();
        RouteSink::new(move |next: Route| *cell.borrow_mut() = next)
    });
    router.navigate(here.clone()).expect("first");
    (router, nav, cell)
}

fn services() -> Route {
    Route::Services {
        host: "ubuntu".to_string(),
    }
}

#[test]
fn back_returns_to_where_it_came_from() {
    let (_router, nav, here) = navigator();

    assert!(!nav.can_go_back(), "nothing visited yet");
    nav.to(services());
    assert_eq!(*here.borrow(), services());

    assert!(nav.back(), "there is somewhere to go");
    assert_eq!(
        *here.borrow(),
        Route::Processes {
            host: "ubuntu".to_string()
        },
        "back publishes the route too, not just installs it"
    );
}

#[test]
fn back_at_the_beginning_says_so_instead_of_moving() {
    let (_router, nav, here) = navigator();
    let before = here.borrow().clone();

    assert!(!nav.back(), "nowhere to go");
    assert_eq!(*here.borrow(), before, "and nothing moved");
}

#[test]
fn forward_undoes_a_back_and_an_ordinary_move_discards_it() {
    let (_router, nav, here) = navigator();

    nav.to(services());
    nav.back();
    assert!(nav.can_go_forward());

    assert!(nav.forward());
    assert_eq!(*here.borrow(), services());

    // Going somewhere new cuts off what was ahead - the same rule a browser
    // follows, and for the same reason: that future no longer exists.
    nav.back();
    nav.to(Route::Processes {
        host: "fedora".to_string(),
    });
    assert!(!nav.can_go_forward(), "the forward stack was cut");
}
