//! What the tool watching an application is told of its windows: what is
//! mounted in each, under what, where it was written, and where its
//! navigators are.

use guinea_app::app::GuineaHost;
use guinea_app::app::roots::Root;
use guinea_core::actor::shape::Declared;
use guinea_core::__private::observer;
use guinea_core::observability::{Event, Navigation, Watching};
use guinea_core::scope::Scope;
use guinea_router::mount::{self, Pass};
use guinea_router::navigator::Navigator;

#[derive(Clone, PartialEq, Debug, guinea_macros::Routable)]
enum Place {
    Home,
    Host { name: String },
}

struct Shell;
struct Page;
struct Other;

fn mount<S: 'static>(parent: Scope) -> Scope {
    let pass = Pass::begin(parent);
    let mounted = pass.mount::<S, ()>(&(), |_| Ok(()));
    pass.end();
    mounted.expect("mounted")
}

/// An application with nothing built in it, and a window of it.
fn window() -> (GuineaHost, Root) {
    let app = GuineaHost::new();
    let root = Root::under(&app.context());
    (app, root)
}

/// The window and the tool watching its tree.
struct Watched {
    watching: Watching,
    root: Root,
    _app: GuineaHost,
}

fn watched((app, root): (GuineaHost, Root)) -> Watched {
    let watching = observer::watch(root.scope(), || {}).expect("nothing else watches");
    Watched {
        watching,
        root,
        _app: app,
    }
}

fn mounted(events: &[Event]) -> Vec<(&'static str, Option<u32>)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Mounted { name, written, .. } => Some((*name, written.map(|written| written.line))),
            _ => None,
        })
        .collect()
}

#[test]
fn what_is_mounted_in_a_window_is_told_under_what_mounted_it() {
    let watched = watched(window());
    watched.watching.take();

    let shell = mount::<Shell>(watched.root.scope());
    let page = mount::<Page>(shell);

    let events = watched.watching.take().events;
    assert_eq!(
        events,
        [
            Event::ScopeOpened { scope: shell.id(), parent: Some(watched.root.scope().id()) },
            Event::Mounted { scope: shell.id(), name: std::any::type_name::<Shell>(), written: None },
            Event::ScopeOpened { scope: page.id(), parent: Some(shell.id()) },
            Event::Mounted { scope: page.id(), name: std::any::type_name::<Page>(), written: None },
        ]
    );
}

#[test]
fn a_page_that_did_not_mount_is_not_told_mounted() {
    let watched = watched(window());
    watched.watching.take();

    let pass = Pass::begin(watched.root.scope());
    let refused = pass.mount::<Page, ()>(&(), |_| Err(anyhow::anyhow!("no")));
    pass.end();

    assert!(refused.is_err());
    assert_eq!(mounted(&watched.watching.take().events), []);
}

#[test]
fn a_mounted_page_says_where_it_was_written() {
    let watched = watched(window());
    watched.watching.take();
    let here = Declared {
        file: file!(),
        line: line!(),
        column: column!(),
        crate_dir: env!("CARGO_MANIFEST_DIR"),
    };

    let pass = Pass::begin(watched.root.scope());
    pass.mount::<Page, ()>(&(), |cx| {
        mount::written(cx.__scope(), Some(here));
        Ok(())
    })
    .expect("mounted");
    pass.end();

    assert_eq!(
        mounted(&watched.watching.take().events),
        [(std::any::type_name::<Page>(), Some(here.line))]
    );
}

#[test]
fn a_page_kept_asleep_is_told_asleep() {
    let watched = watched(window());
    let pass = Pass::begin(watched.root.scope());
    let kept = pass.mount::<Page, ()>(&(), |_| Ok(())).expect("mounted");
    pass.keep(kept);
    pass.end();
    watched.watching.take();

    let pass = Pass::begin(watched.root.scope());
    pass.mount::<Other, ()>(&(), |_| Ok(())).expect("mounted");
    pass.end();

    let events = watched.watching.take().events;
    assert!(events.contains(&Event::Slept { scope: kept.id() }), "{events:#?}");
}

#[test]
fn a_navigator_is_told_where_it_opened_and_where_it_moved() {
    let watched = watched(window());
    watched.watching.take();

    let nav = Navigator::open(watched.root.scope(), Place::Home);
    let _moved = nav.to(Place::Host { name: "db-01".into() });

    let scope = watched.root.scope().id();
    let at = |route: &str, back| Event::Navigated {
        scope,
        navigation: Navigation {
            routes: std::any::type_name::<Place>(),
            route: route.into(),
            back,
            forward: 0,
            pending: None,
            failure: None,
        },
    };
    assert_eq!(
        watched.watching.take().events,
        [at("Home", 0), at("Host { name: \"db-01\" }", 1)]
    );
}

#[test]
fn a_tool_that_starts_watching_is_told_what_is_mounted_and_where_the_navigators_are() {
    let (app, root) = window();
    let shell = mount::<Shell>(root.scope());
    let nav = Navigator::open(root.scope(), Place::Home);
    let _moved = nav.to(Place::Host { name: "db-01".into() });

    let watched = watched((app, root));
    let events = watched.watching.take().events;

    let navigated = events.iter().find_map(|event| match event {
        Event::Navigated { navigation, .. } => Some(navigation.route.clone()),
        _ => None,
    });
    assert_eq!(
        (mounted(&events), navigated),
        (
            vec![(std::any::type_name::<Shell>(), None)],
            Some("Host { name: \"db-01\" }".to_string())
        ),
        "{events:#?}"
    );
    assert!(events.contains(&Event::ScopeOpened {
        scope: shell.id(),
        parent: Some(watched.root.scope().id())
    }));
}
