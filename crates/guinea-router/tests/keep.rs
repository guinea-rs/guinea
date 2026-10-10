//! A `keep` segment: left, it sleeps; reached again, it wakes as it was.
//!
//! What it saves is everything a fresh install would throw away - state, actors,
//! what it was in the middle of - and what it costs is that it must not act
//! while nobody can see it. These say both halves, and where keeping stops: a
//! sleeping segment is only ever woken under the scopes it was installed
//! under, and it goes when they go.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::actor::event_bus::Event;
use guinea_core::guard::Verdict;
use guinea_core::scope::{DropGuard, Scope};
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{RouteChain, Router, SegmentEntry};

thread_local! {
    static SAID: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn say(what: impl Into<String>) {
    SAID.with(|said| said.borrow_mut().push(what.into()));
}

fn said() -> Vec<String> {
    SAID.with(|said| said.take())
}

struct Gone(&'static str);

impl Drop for Gone {
    fn drop(&mut self) {
        say(format!("gone {}", self.0));
    }
}

#[derive(Clone)]
struct Ping;

impl Event for Ping {}

fn installed(cx: &FeatureInitContext, name: &'static str) {
    say(format!("install {name}"));
    cx.scope.own(DropGuard(Gone(name)));
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = ();

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        installed(cx, "Shell");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

/// Kept where the routes say so. Hears `Ping`, and says when it wakes and when
/// it is asked whether it may go.
struct Area;

impl Layout for Area {
    type Params = String;
    type Installs = ();

    fn install(cx: &FeatureInitContext, host: &String) -> anyhow::Result<()> {
        installed(cx, "Area");

        let host = host.clone();
        cx.subscribe(move |_: Ping| say(format!("Area on {host} heard a ping")));
        cx.on_wake(|| say("Area woke"));
        cx.on_leave(|| {
            say("Area asked");
            Verdict::Allow
        });
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct List;

impl Page for List {
    type Params = String;
    type Installs = ();

    fn install(cx: &FeatureInitContext, _host: &String) -> anyhow::Result<()> {
        installed(cx, "List");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Other;

impl Page for Other {
    type Params = ();
    type Installs = ();

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        installed(cx, "Other");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

struct Elsewhere;

impl Page for Elsewhere {
    type Params = ();
    type Installs = ();

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        installed(cx, "Elsewhere");
        Ok(())
    }

    fn view(_cx: &mut HeadlessCx<Self>) {}
}

const LIST: [SegmentEntry<Headless>; 3] = [
    layout_entry::<Shell>(),
    layout_entry::<Area>().kept(),
    segment_entry::<List>(),
];
const OTHER: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Other>()];
const ELSEWHERE: [SegmentEntry<Headless>; 1] = [segment_entry::<Elsewhere>()];
const UNSHELLED: [SegmentEntry<Headless>; 3] = [
    segment_entry::<Elsewhere>(),
    layout_entry::<Area>().kept(),
    segment_entry::<List>(),
];

#[derive(Clone)]
enum Route {
    List(String),
    Other,
    Elsewhere,
}

impl RouteChain<Headless> for Route {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        match self {
            Route::List(_) => &LIST,
            Route::Other => &OTHER,
            Route::Elsewhere => &ELSEWHERE,
        }
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        match self {
            Route::List(host) => vec![Box::new(()), Box::new(host.clone()), Box::new(host.clone())],
            Route::Other => vec![Box::new(()), Box::new(())],
            Route::Elsewhere => vec![Box::new(())],
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Route::List(_) => "List",
            Route::Other => "Other",
            Route::Elsewhere => "Elsewhere",
        }
    }
}

/// A chain whose `keep` layout sits under a segment that goes.
#[derive(Clone)]
struct Unshelled;

impl RouteChain<Headless> for Unshelled {
    fn chain(&self) -> &'static [SegmentEntry<Headless>] {
        &UNSHELLED
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        vec![Box::new(()), Box::new("a".to_string()), Box::new("a".to_string())]
    }

    fn name(&self) -> &'static str {
        "Unshelled"
    }
}

fn router() -> Rc<Router<Headless>> {
    said();
    Rc::new(Router::<Headless>::new(FeatureHost::detached()))
}

fn go(router: &Rc<Router<Headless>>, route: impl RouteChain<Headless> + 'static) {
    assert!(router.navigate(route).expect("navigate").is_done());
}

fn area(router: &Router<Headless>) -> Scope {
    router.scope_at(1).expect("the area is mounted")
}

#[test]
fn a_kept_segment_comes_back_as_it_was_without_installing_again() {
    let router = router();
    go(&router, Route::List("a".into()));
    let before = area(&router);
    said();

    go(&router, Route::Other);
    assert_eq!(said(), ["gone List", "install Other"], "only the list goes; the area sleeps");

    go(&router, Route::List("a".into()));
    assert_eq!(said(), ["gone Other", "install List", "Area woke"]);
    assert_eq!(before, area(&router), "the area came back as another scope");
}

#[test]
fn a_sleeping_segment_hears_nothing_and_is_not_told_later() {
    let router = router();
    go(&router, Route::List("a".into()));
    go(&router, Route::Other);
    said();

    router.host().event_bus().publish(Ping);
    assert_eq!(said(), Vec::<String>::new(), "the area heard a ping in its sleep");

    go(&router, Route::List("a".into()));
    router.host().event_bus().publish(Ping);
    assert_eq!(
        said(),
        ["gone Other", "install List", "Area woke", "Area on a heard a ping"],
        "what it slept through came to it on waking, or it stayed deaf"
    );
}

#[test]
fn leaving_a_kept_segment_does_not_ask_it_whether_it_may_go() {
    let router = router();
    go(&router, Route::List("a".into()));
    said();

    go(&router, Route::Other);
    assert!(!said().contains(&"Area asked".to_string()), "it is not going anywhere");
}

#[test]
fn a_sleeping_segment_reached_with_another_capture_is_installed_fresh() {
    let router = router();
    go(&router, Route::List("a".into()));
    go(&router, Route::Other);
    said();

    go(&router, Route::List("b".into()));
    assert_eq!(said(), ["gone Other", "gone Area", "install Area", "install List"]);
}

#[test]
fn a_sleeping_segment_goes_before_what_it_slept_under() {
    let router = router();
    go(&router, Route::List("a".into()));
    go(&router, Route::Other);
    said();

    go(&router, Route::Elsewhere);
    assert_eq!(said(), ["gone Other", "gone Area", "gone Shell", "install Elsewhere"]);
}

#[test]
fn a_kept_segment_under_one_that_goes_goes_with_it() {
    let router = router();
    go(&router, Unshelled);
    said();

    go(&router, Route::Other);
    assert_eq!(
        said(),
        [
            "Area asked",
            "gone List",
            "gone Area",
            "gone Elsewhere",
            "install Shell",
            "install Other"
        ]
    );
}

#[test]
fn what_sleeps_is_torn_down_with_the_router() {
    let router = router();
    go(&router, Route::List("a".into()));
    go(&router, Route::Other);
    said();

    drop(router);
    assert_eq!(said(), ["gone Area", "gone Other", "gone Shell"]);
}
