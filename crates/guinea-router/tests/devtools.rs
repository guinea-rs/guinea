//! What devtools read from a live router.

use std::any::Any;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use std::cell::RefCell;

use guinea_core::actor::{Cx, Handler};
use guinea_core::observability::changes::{self, Change};
use guinea_core::feature::Bound;
use guinea_core::scope::Reducer;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::observability;
use guinea_router::router::{RouteChain, Router, SegmentEntry};

#[derive(Default, Clone, Debug)]
struct Counter {
    seen: u32,
}

impl Reducer for Counter {
    type Update = u32;

    fn reduce(&mut self, seen: u32) {
        self.seen = seen;
    }
}

struct Frame;

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

impl Handler<Wind> for Clock {
    fn handle(&mut self, _: Wind, _cx: Cx<Self, Wind>) {
        self.0 += 1;
    }
}

impl Layout for Frame {
    type Params = ();
    type Installs = ();

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        ctx.spawn(Clock(0));
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Detail;

impl Page for Detail {
    type Params = ();
    type Installs = Bound<Counter>;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Bound<Counter>> {
        Ok(ctx.state::<Counter>().seed(Counter { seen: 3 }).plain())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const CHAIN: [SegmentEntry<Headless>; 2] = [layout_entry::<Frame>(), segment_entry::<Detail>()];

struct Route {
    id: u32,
}

impl RouteChain<Headless> for Route {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        &CHAIN
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        vec![Box::new(()), Box::new(())]
    }

    fn name(&self) -> &'static str {
        "Detail"
    }

    fn describe(&self) -> String {
        format!("Route {{ id: {} }}", self.id)
    }
}

#[test]
fn a_router_shows_up_once_it_has_navigated_and_goes_when_dropped() {
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    let before = observability::routers().len();

    router.navigate(Route { id: 7 }).expect("navigate");
    router.navigate(Route { id: 8 }).expect("navigate again");

    let views = observability::routers();
    assert_eq!(views.len(), before + 1, "registered once, not per navigation");

    let view = views.last().unwrap();
    assert_eq!(view.root, router.root());
    assert_eq!(view.backend, "Headless");
    assert_eq!(view.route.as_deref(), Some("Route { id: 8 }"));

    let names: Vec<&str> = view.segments.iter().map(|segment| segment.name).collect();
    assert_eq!(names, ["Frame", "Detail"]);

    let detail = &view.segments[1].states;
    assert_eq!(detail.len(), 1);
    assert!(detail[0].type_name.ends_with("Counter"));
    assert!(detail[0].state.contains("seen: 3"), "{}", detail[0].state);

    drop(views);
    drop(router);
    assert_eq!(observability::routers().len(), before);
}

#[test]
fn devtools_hear_a_router_open_and_close_and_read_it_alone() {
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    let root = router.root().get();

    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    changes::watch(move |change| sink.borrow_mut().push(change.clone()));

    router.navigate(Route { id: 1 }).expect("navigate");
    router.navigate(Route { id: 2 }).expect("navigate again");
    let read = observability::router(root).map(|view| view.route);
    drop(router);
    changes::stop_watching();

    assert_eq!(read, Some(Some("Route { id: 2 }".to_string())));
    assert!(observability::router(root).is_none(), "read after it was gone");

    let routers: Vec<Change> = seen
        .take()
        .into_iter()
        .filter(|change| {
            matches!(
                change,
                Change::RouterOpened { .. } | Change::RouterClosed { .. }
            )
        })
        .collect();
    assert_eq!(
        routers,
        [Change::RouterOpened { root }, Change::RouterClosed { root }]
    );
}

#[derive(Clone, Debug, guinea_macros::Event)]
struct Resized;

#[test]
fn a_router_view_says_who_hears_its_window_s_bus() {
    let host = FeatureHost::detached();
    let subscribed_at = line!() + 1;
    let _hears = host.event_bus().subscribe_fn(|_: Resized| {});
    let router = Rc::new(Router::<Headless>::new(host));
    router.navigate(Route { id: 1 }).expect("navigate");
    let root = router.root().get();

    let listening = observability::router(root).expect("the window is open").bus;

    let resized = listening
        .iter()
        .find(|listening| listening.event.ends_with("Resized"))
        .map(|listening| listening.listeners.clone());
    let places: Option<Vec<(&str, u32)>> = resized.map(|listeners| {
        listeners
            .iter()
            .filter_map(|listener| match listener.by {
                guinea_core::actor::event_bus::HeardBy::Callback(at) => Some((at.file(), at.line())),
                _ => None,
            })
            .collect()
    });
    assert_eq!(places, Some(vec![(file!(), subscribed_at)]), "{listening:#?}");
}

#[test]
fn one_actor_of_one_window_is_read_by_its_id() {
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached()));
    router.navigate(Route { id: 1 }).expect("navigate");
    let root = router.root().get();

    let listed = observability::routers()
        .into_iter()
        .find(|view| view.root.get() == root)
        .and_then(|view| view.actors.into_iter().find(|actor| actor.type_name.ends_with("Clock")))
        .map(|actor| actor.id);
    let read = listed.and_then(|id| observability::actor(Some(root), id)).map(|actor| actor.id);

    assert!(listed.is_some(), "the clock was spawned");
    assert_eq!(read, listed);
    assert!(observability::actor(Some(root), usize::MAX).is_none());
    assert!(
        listed.and_then(|id| observability::actor(None, id)).is_none(),
        "the application's actors are not the window's"
    );
}
