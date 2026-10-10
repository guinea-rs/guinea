//! A segment whose `install` fails: the navigation does not happen, and the
//! router stays where it was instead of being left with nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::scope::Reducer;
use guinea_macros::routes;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page};
use guinea_router::router::{NavigateHandle, RouteSink, Router};

thread_local! {
    static INSTALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static BROKEN: Cell<bool> = const { Cell::new(false) };
}

fn installed(what: String) {
    INSTALLS.with(|installs| installs.borrow_mut().push(what));
}

fn installs() -> Vec<String> {
    INSTALLS.with(|installs| installs.borrow().clone())
}

#[derive(Default, Clone, PartialEq, Debug)]
struct Draft {
    text: String,
}

impl Reducer for Draft {
    type Update = String;

    fn reduce(&mut self, text: String) {
        self.text = text;
    }
}

struct Shell;

impl Layout for Shell {
    type Params = ShellParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<()> {
        installed("Shell".to_string());
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Host;

impl Page for Host {
    type Params = HostParams;
    type Installs = ();

    fn install(ctx: &FeatureInitContext, params: &HostParams) -> anyhow::Result<()> {
        installed(format!("Host {}", params.name));
        ctx.scope.push::<Draft>(format!("draft on {}", params.name));
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Agent;

impl Page for Agent {
    type Params = AgentParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &AgentParams) -> anyhow::Result<()> {
        if BROKEN.with(Cell::get) {
            anyhow::bail!("the agent is not reachable");
        }
        installed("Agent".to_string());
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Shell) {
            page(Host) link("/host/:name") { name: String }
            page(Agent) link("/agent")
        }
    }
}

fn router() -> Rc<Router<Headless>> {
    INSTALLS.with(|installs| installs.borrow_mut().clear());
    BROKEN.with(|broken| broken.set(true));

    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    router
        .navigate(Route::Host {
            name: "ubuntu".to_string(),
        })
        .expect("the first route");
    router
}

#[test]
fn a_failed_install_leaves_the_router_where_it_was() {
    let router = router();

    let error = router.navigate(Route::Agent {}).map(|_| ()).expect_err("the agent failed");
    assert!(format!("{error:#}").contains("the agent is not reachable"));

    assert_eq!(
        router.current_route::<Route>(),
        Some(Route::Host {
            name: "ubuntu".to_string()
        })
    );
    assert_eq!(
        installs(),
        ["Shell", "Host ubuntu", "Host ubuntu"],
        "the page it left went back up; the layout it shared never came down"
    );
    let draft = router.active_scope().expect("a chain stands").state::<Draft>();
    assert_eq!(draft.borrow().text, "draft on ubuntu");
}

#[test]
fn a_failed_navigation_from_a_handle_is_not_a_panic_and_not_an_arrival() {
    let router = router();
    let published = Rc::new(RefCell::new(Vec::new()));
    let publishing = published.clone();
    let nav = NavigateHandle::new(
        router.clone(),
        RouteSink::new(move |route: Route| publishing.borrow_mut().push(route)),
    );

    nav.to(Route::Agent {});

    assert!(published.borrow().is_empty());
    assert!(!nav.can_go_back(), "a place never left is not history");
    assert_eq!(
        router.current_route::<Route>(),
        Some(Route::Host {
            name: "ubuntu".to_string()
        })
    );

    BROKEN.with(|broken| broken.set(false));
    nav.to(Route::Agent {});
    assert_eq!(*published.borrow(), [Route::Agent {}]);
}
