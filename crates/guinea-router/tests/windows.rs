//! What devtools read of an open window: what is mounted in it, where its
//! navigators are, and who hears its bus.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::feature::FeatureHost;
use guinea_core::actor::shape::Declared;
use guinea_core::observability::changes::{self, Change};
use guinea_core::scope::Scope;
use guinea_router::mount::{self, Pass};
use guinea_router::navigator::Navigator;
use guinea_router::observability;

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

fn root(host: &FeatureHost) -> u64 {
    guinea_app::app::roots::of(host.scope()).expect("a window is a root").get()
}

#[test]
fn a_window_lists_what_is_mounted_in_it_each_under_what_mounted_it() {
    let host = FeatureHost::detached();
    let shell = mount::<Shell>(host.scope());
    mount::<Page>(shell);

    let view = observability::window(root(&host));

    let mounted: Vec<(&str, usize)> = view
        .map(|view| view.mounted.iter().map(|mounted| (mounted.name, mounted.depth)).collect())
        .unwrap_or_default();
    assert_eq!(mounted, [("Shell", 0), ("Page", 1)]);
}

#[test]
fn a_page_kept_asleep_is_listed_asleep() {
    let host = FeatureHost::detached();

    let pass = Pass::begin(host.scope());
    let kept = pass.mount::<Page, ()>(&(), |_| Ok(())).expect("mounted");
    pass.keep(kept);
    pass.end();
    let pass = Pass::begin(host.scope());
    pass.mount::<Other, ()>(&(), |_| Ok(())).expect("mounted");
    pass.end();

    let view = observability::window(root(&host));

    let awake: Vec<(&str, bool)> = view
        .map(|view| view.mounted.iter().map(|mounted| (mounted.name, mounted.awake)).collect())
        .unwrap_or_default();
    assert_eq!(awake, [("Page", false), ("Other", true)]);
}

#[test]
fn a_window_lists_where_its_navigator_is() {
    let host = FeatureHost::detached();
    let nav = Navigator::open(host.scope(), Place::Home);
    let _moved = nav.to(Place::Host { name: "db-01".into() });

    let view = observability::window(root(&host));

    let navigators: Vec<(String, usize, usize)> = view
        .map(|view| {
            view.navigators
                .into_iter()
                .map(|nav| (nav.route, nav.back, nav.forward))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(navigators, [("Host { name: \"db-01\" }".to_string(), 1, 0)]);
}

#[test]
fn a_mounted_page_says_where_it_was_written() {
    let host = FeatureHost::detached();
    let page = mount::<Page>(host.scope());
    let here = Declared {
        file: file!(),
        line: line!(),
        column: column!(),
        crate_dir: env!("CARGO_MANIFEST_DIR"),
    };
    mount::written(page, Some(here));

    let view = observability::window(root(&host));

    let written = view.and_then(|view| view.mounted.first().and_then(|mounted| mounted.written));
    assert_eq!(written.map(|written| written.line), Some(here.line));
}

#[test]
fn every_open_window_is_listed_and_one_that_closed_is_not() {
    let first = FeatureHost::detached();
    let second = FeatureHost::detached();
    let (first_root, second_root) = (root(&first), root(&second));

    let listed: Vec<u64> = observability::windows().iter().map(|view| view.root.get()).collect();
    assert!(listed.contains(&first_root) && listed.contains(&second_root), "{listed:?}");

    drop(first);

    assert!(observability::window(first_root).is_none());
    assert!(observability::window(second_root).is_some());
}

#[test]
fn a_tool_is_told_when_a_window_opens_and_closes() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    changes::watch(move |change| sink.borrow_mut().push(change.clone()));

    let host = FeatureHost::detached();
    let root = root(&host);
    drop(host);
    changes::stop_watching();

    let windows: Vec<Change> = seen
        .take()
        .into_iter()
        .filter(|change| matches!(change, Change::RootOpened { .. } | Change::RootClosed { .. }))
        .collect();
    assert_eq!(windows, [Change::RootOpened { root }, Change::RootClosed { root }]);
}

#[derive(Default, Clone, Debug)]
struct Counter {
    seen: u32,
}

#[guinea_macros::reducer]
fn counter(this: &mut Counter, seen: u32) {
    this.seen = seen;
}

#[derive(Debug)]
struct Clock(u32);

struct Wind;

guinea_macros::actor! {
    Clock {
        handlers {
            Wind
        }
    }
}

impl guinea_core::actor::Handler<Wind> for Clock {
    fn handle(&mut self, _: Wind, _cx: guinea_core::actor::Cx<Self, Wind>) {
        self.0 += 1;
    }
}

#[test]
fn a_mounted_page_lists_the_state_it_installed() {
    let host = FeatureHost::detached();
    let pass = Pass::begin(host.scope());
    pass.mount::<Page, ()>(&(), |cx| {
        cx.scope.hold_installed(cx.state::<Counter>().seed(Counter { seen: 3 }).plain());
        Ok(())
    })
    .expect("mounted");
    pass.end();

    let view = observability::window(root(&host));

    let states: Vec<(String, String)> = view
        .into_iter()
        .flat_map(|view| view.mounted)
        .flat_map(|mounted| mounted.states)
        .map(|state| (state.type_name.to_string(), state.state.preview))
        .collect();
    assert_eq!(states.len(), 1, "{states:?}");
    assert!(states[0].0.ends_with("Counter") && states[0].1.contains("seen: 3"), "{states:?}");
}

#[test]
fn one_actor_of_one_window_is_read_by_its_id() {
    let host = FeatureHost::detached();
    let pass = Pass::begin(host.scope());
    pass.mount::<Shell, ()>(&(), |cx| {
        cx.spawn(Clock(0));
        Ok(())
    })
    .expect("mounted");
    pass.end();
    let root = root(&host);

    let listed = observability::window(root)
        .and_then(|view| view.actors.into_iter().find(|actor| actor.type_name.ends_with("Clock")))
        .map(|actor| actor.id);
    let read = listed.and_then(|id| observability::actor(root, id)).map(|actor| actor.id);

    assert!(listed.is_some(), "the clock was spawned");
    assert_eq!(read, listed);
    assert!(observability::actor(root, usize::MAX).is_none());
}

#[derive(Clone, Debug, guinea_macros::Event)]
struct Resized;

#[test]
fn a_window_says_who_hears_the_bus_of_its_tree() {
    let host = FeatureHost::detached();
    let subscribed_at = line!() + 1;
    let _hears = host.scope().bus().subscribe_fn(|_: Resized| {});

    let listening = observability::window(root(&host)).map(|view| view.bus).unwrap_or_default();

    let places: Option<Vec<(&str, u32)>> = listening
        .iter()
        .find(|listening| listening.event.ends_with("Resized"))
        .map(|listening| {
            listening
                .listeners
                .iter()
                .filter_map(|listener| match listener.by {
                    guinea_core::actor::event_bus::HeardBy::Callback(at) => Some((at.file(), at.line())),
                    _ => None,
                })
                .collect()
        });
    assert_eq!(places, Some(vec![(file!(), subscribed_at)]), "{listening:#?}");
}
