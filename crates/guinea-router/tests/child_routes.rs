//! What a layout can offer as a menu: its children, each as the route that
//! reaches it from here, and which one is current.

use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::actor::UiThreadToken;
use guinea_macros::routes;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page};
use guinea_router::router::{ChildRoute, Router};

macro_rules! layout {
    ($layout:ident, $params:ident) => {
        pub struct $layout;

        impl Layout for $layout {
            type Params = $params;
            type Installs = ();

            fn install(_ctx: &FeatureInitContext, _params: &$params) -> anyhow::Result<()> {
                Ok(())
            }

            fn view(cx: &mut HeadlessCx<Self>) {
                cx.outlet();
            }
        }
    };
}

macro_rules! page {
    ($page:ident, $params:ident) => {
        pub struct $page;

        impl Page for $page {
            type Params = $params;
            type Installs = ();

            fn install(_ctx: &FeatureInitContext, _params: &$params) -> anyhow::Result<()> {
                Ok(())
            }

            fn view(_cx: &mut HeadlessCx<Self>) {}
        }
    };
}

layout!(Shell, ShellParams);
layout!(Area, AreaParams);
page!(A, AParams);
page!(B, BParams);
page!(C, CParams);
page!(D, DParams);
page!(E, EParams);

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        layout(Shell) {
            page(A)
            layout(Area) {
                page(B)
                page(C)
            }
            page(D) { id: u32 }
        }
        page(E)
    }
}

fn router() -> Rc<Router<Headless>> {
    let token = UiThreadToken::dangerously_create_token_unchecked();
    Rc::new(Router::<Headless>::new(FeatureHost::detached(token)))
}

fn child(route: Route, current: bool) -> ChildRoute<Route> {
    ChildRoute { route, current }
}

#[test]
fn a_layout_s_children_are_its_pages_and_the_first_page_of_each_layout_under_it() {
    let router = router();
    router.navigate(Route::A {}).expect("to a");

    assert_eq!(
        router.child_routes::<Route>(0),
        [child(Route::A {}, true), child(Route::B {}, false)],
        "`D` needs an `id` the shell does not carry, so it is not offered"
    );
}

#[test]
fn a_child_layout_is_current_while_any_page_under_it_is() {
    let router = router();
    router.navigate(Route::C {}).expect("to c");

    assert_eq!(
        router.child_routes::<Route>(0),
        [child(Route::A {}, false), child(Route::B {}, true)]
    );
    assert_eq!(
        router.child_routes::<Route>(1),
        [child(Route::B {}, false), child(Route::C {}, true)]
    );
}

mod carried {
    use super::*;

    layout!(Machine, MachineParams);
    page!(Processes, ProcessesParams);
    page!(Services, ServicesParams);
    page!(Process, ProcessParams);

    routes! {
        backend = guinea_router::headless::Headless,
        MachineRoute {
            layout(Machine) {
                page(Processes) { context: String }
                page(Services) { context: String }
                page(Process) { context: String, pid: u32 }
            }
        }
    }

    #[test]
    fn a_child_is_reached_with_what_its_layout_carries() {
        let router = router();
        router
            .navigate(MachineRoute::Services { context: "ubuntu".into() })
            .expect("to services");

        assert_eq!(
            router.child_routes::<MachineRoute>(0),
            [
                ChildRoute {
                    route: MachineRoute::Processes { context: "ubuntu".into() },
                    current: false,
                },
                ChildRoute {
                    route: MachineRoute::Services { context: "ubuntu".into() },
                    current: true,
                },
            ]
        );
    }
}
