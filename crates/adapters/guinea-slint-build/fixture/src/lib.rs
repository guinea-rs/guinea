//! An application of one window, its `Navigator` global written by
//! `guinea-slint-build` from [`routes`].

slint::include_modules!();

pub mod routes;

use guinea::slint::{Layout, LayoutCx, Page, layout, page};
use routes::{ProcessesRoute, Route};

include!(concat!(env!("OUT_DIR"), "/navigator.rs"));

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn mount(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Route>();
        cx.outlet(&nav, |route, cx| match route {
            Route::Home | Route::Host { .. } => cx.mount::<Home>(&()),
            Route::Processes(ProcessesRoute::List) => cx.mount::<Processes>(&None),
            Route::Processes(ProcessesRoute::One { pid }) => cx.mount::<Processes>(&Some(*pid)),
        });
    }
}

#[derive(Default)]
pub struct Home;

#[page]
impl Page for Home {}

#[derive(Default)]
pub struct Processes;

#[page]
impl Page for Processes {
    type Params = Option<u32>;
}

#[cfg(test)]
mod tests {
    use guinea::app::GuineaHost;
    use guinea::app::roots::Root;
    use guinea::slint::{Opened, Window};
    use slint::ComponentHandle;

    use super::*;

    struct Open {
        component: App,
        nav: guinea::slint::Navigator<Route>,
        _root: Root,
        _opened: Opened,
        _app: GuineaHost,
    }

    fn at(route: Route) -> Open {
        i_slint_backend_testing::init_no_event_loop();

        let app = GuineaHost::new();
        let root = Root::under(&app.context());
        let component = App::new().expect("the window");
        let opened = Window::of::<Shell>(component.clone_strong())
            .at(route)
            .open(root.scope())
            .expect("the window opens");
        let nav = guinea::slint::Navigator::find(root.scope()).expect("its navigator");

        Open {
            component,
            nav,
            _root: root,
            _opened: opened,
            _app: app,
        }
    }

    fn host() -> Route {
        Route::Host {
            name: "db-01".into(),
            port: 5432,
        }
    }

    #[test]
    fn the_global_starts_where_the_window_does_with_that_place_s_parameters() {
        let window = at(host());
        let global = window.component.global::<Navigator>();

        assert_eq!(global.get_route(), RouteId::Host);
        assert_eq!(global.get_host_params().name, "db-01");
        assert_eq!(global.get_host_params().port, 5432);
        assert!(!global.get_can_go_back());
    }

    #[test]
    fn a_go_callback_takes_the_window_to_a_nested_place() {
        let window = at(host());
        let global = window.component.global::<Navigator>();

        global.invoke_go_processes_one(ProcessesOneParams { pid: 7 });

        assert_eq!(window.nav.current(), Some(Route::Processes(ProcessesRoute::One { pid: 7 })));
        assert_eq!(global.get_route(), RouteId::ProcessesOne);
        assert_eq!(global.get_processes_one_params().pid, 7);
        assert!(global.get_can_go_back());
    }

    #[test]
    fn back_and_forward_on_the_global_move_the_window() {
        let window = at(host());
        let global = window.component.global::<Navigator>();
        global.invoke_go_home();

        global.invoke_back();
        assert_eq!(window.nav.current(), Some(host()));
        assert!(global.get_can_go_forward());

        global.invoke_forward();
        assert_eq!(global.get_route(), RouteId::Home);
    }
}
