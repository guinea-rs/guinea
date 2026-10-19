//! A feed: one feature writes the latest value, and what is installed under
//! it follows - handed each value once, and nothing while it sleeps.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use guinea::app::Harness;
use guinea_core::notify;
use guinea::prelude::*;

#[derive(Debug, PartialEq)]
pub struct Report(pub u32);

#[derive(Clone, Debug)]
pub struct Greeting(pub &'static str);

/// Every report a follower was handed, in order.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Seen(pub Vec<u32>);

#[reducer]
fn saw(this: &mut Seen, report: u32) {
    this.0.push(report);
}

/// Every time the feeding feature heard its feed's following start or stop.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Heard(pub Vec<bool>);

#[reducer]
fn heard(this: &mut Heard, followed: bool) {
    this.0.push(followed);
}

thread_local! {
    static WRITER: RefCell<Option<FeedWriter<Report>>> = const { RefCell::new(None) };
    static FIRST_OPENED_AT: Cell<u32> = const { Cell::new(0) };
}

fn set(report: impl Into<Arc<Report>>) {
    let report = report.into();
    WRITER.with_borrow(|writer| writer.as_ref().expect("the feed was opened").set(report));
}

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .unwrap_or_default()
}

fn installed(result: anyhow::Result<impl Sized>) -> Result<(), String> {
    result.map(|_| ()).map_err(|error| format!("{error:#}"))
}

fn sleep(segment: &guinea::app::Segment<'_>) {
    segment.scope().sleep();
}

fn wake(segment: &guinea::app::Segment<'_>) {
    segment.scope().wake();
}

mod scanning {
    use super::*;

    feature! {
        pub Scanning {
            exports { Heard }
            feeds { Report }
        }
    }

    #[installs]
    fn scanning(cx: &Install) -> anyhow::Result<Scanning> {
        let heard = cx.state::<Heard>().plain();
        cx.answers(move |followed: Followed<Report>| heard.push(followed.0));

        let report = cx.feed::<Report>();
        report.set(Report(1));
        WRITER.set(Some(report.clone()));

        Ok(Scanning(heard, report.feed()))
    }
}

mod producing {
    use super::*;

    #[derive(Debug)]
    pub struct Producer {
        heard: Push<Heard>,
    }

    actor! {
        Producer {
            handlers { Followed<Report> }
        }
    }

    #[handler]
    fn followed(In(this, followed, _): In<Producer, Followed<Report>>) {
        this.heard.send(followed.0);
    }

    feature! {
        pub Producing {
            exports { Heard }
            feeds { Report }
        }
    }

    #[installs]
    fn producing(cx: &Install) -> anyhow::Result<Producing> {
        let (heard, _) = cx.state::<Heard>().driven_by(|heard| Producer { heard });
        let report = cx.feed::<Report>();

        Ok(Producing(heard, report.feed()))
    }
}

mod twice {
    use super::*;

    feature! {
        pub Twice {}
    }

    #[installs]
    fn twice(cx: &Install) -> anyhow::Result<Twice> {
        let (_first, at) = (cx.feed::<Report>(), line!());
        FIRST_OPENED_AT.set(at);
        let _second = cx.feed::<Report>();

        Ok(Twice)
    }
}

mod watching {
    use super::*;

    feature! {
        pub Watching {
            exports { Seen }
        }
    }

    #[installs]
    fn watching(cx: &Install,report: Feed<Report>) -> anyhow::Result<Watching> {
        let seen = cx.state::<Seen>().plain();
        cx.follow(report, move |report| seen.push(report.0));

        Ok(Watching(seen))
    }
}

mod glancing {
    use super::*;

    feature! {
        pub Glancing {
            exports { Seen }
        }
    }

    #[installs]
    fn glancing(cx: &Install,report: Option<Feed<Report>>) -> anyhow::Result<Glancing> {
        let seen = cx.state::<Seen>().plain();
        if let Some(report) = report.and_then(|report| report.get()) {
            seen.push(report.0);
        }

        Ok(Glancing(seen))
    }
}

mod needing {
    use super::*;

    thread_local! {
        pub static RAN: Cell<bool> = const { Cell::new(false) };
    }

    feature! {
        pub Needing {}
    }

    #[installs]
    fn needing(
        _cx: &Install,
        _greeting: Greeting,
        _report: Feed<Report>,
    ) -> anyhow::Result<Needing> {
        RAN.set(true);
        Ok(Needing)
    }
}

mod beside {
    use super::*;

    feature! {
        pub Beside {
            exports { Seen }
        }
    }

    #[installs]
    fn beside(cx: &Install) -> anyhow::Result<Beside> {
        let scanning = cx.install::<scanning::Scanning>(&())?;
        let seen = cx.state::<Seen>().plain();
        cx.follow(scanning.1, move |report| seen.push(report.0));

        Ok(Beside(seen))
    }
}

#[guinea::test(iterations = 1)]
fn a_follower_below_is_handed_the_value_the_feed_has(h: &mut Harness) {
    h.install::<scanning::Scanning>(&()).unwrap();

    let page = h.child();
    assert_eq!(installed(page.install::<watching::Watching>(&())), Ok(()));

    assert_eq!(page.state::<Seen>().0, [1]);
}

#[guinea::test(iterations = 1)]
fn a_follower_is_handed_each_value_once(h: &mut Harness) {
    h.install::<scanning::Scanning>(&()).unwrap();
    let page = h.child();
    page.install::<watching::Watching>(&()).unwrap();

    notify::turn(|| {
        set(Report(2));
        set(Report(3));
    });
    set(Report(4));

    let same = Arc::new(Report(5));
    set(same.clone());
    set(same);

    assert_eq!(page.state::<Seen>().0, [1, 3, 4, 5]);
}

#[guinea::test(iterations = 1)]
fn a_sleeping_follower_is_told_nothing_and_on_waking_only_where_the_feed_is(h: &mut Harness) {
    h.install::<scanning::Scanning>(&()).unwrap();
    let page = h.child();
    page.install::<watching::Watching>(&()).unwrap();

    sleep(&page);
    set(Report(2));
    set(Report(3));
    assert_eq!(page.state::<Seen>().0, [1]);

    wake(&page);
    assert_eq!(page.state::<Seen>().0, [1, 3]);

    sleep(&page);
    wake(&page);
    assert_eq!(page.state::<Seen>().0, [1, 3]);
}

#[guinea::test(iterations = 1)]
fn the_feed_hears_when_its_first_awake_follower_comes_and_its_last_goes(h: &mut Harness) {
    h.install::<scanning::Scanning>(&()).unwrap();
    assert_eq!(h.state::<Heard>().0, [] as [bool; 0]);

    let first = h.child();
    first.install::<watching::Watching>(&()).unwrap();
    let second = h.child();
    second.install::<watching::Watching>(&()).unwrap();
    assert_eq!(h.state::<Heard>().0, [true]);

    sleep(&first);
    assert_eq!(h.state::<Heard>().0, [true]);
    sleep(&second);
    assert_eq!(h.state::<Heard>().0, [true, false]);

    wake(&first);
    assert_eq!(h.state::<Heard>().0, [true, false, true]);

    first.leave();
    assert_eq!(h.state::<Heard>().0, [true, false, true, false]);

    second.leave();
    assert_eq!(h.state::<Heard>().0, [true, false, true, false]);
}

#[guinea::test(iterations = 1)]
fn an_actor_hears_its_feed_followed(h: &mut Harness) {
    h.install::<producing::Producing>(&()).unwrap();

    let page = h.child();
    page.install::<watching::Watching>(&()).unwrap();
    h.settled();
    assert_eq!(h.state::<Heard>().0, [true]);

    page.leave();
    h.settled();
    assert_eq!(h.state::<Heard>().0, [true, false]);
}

#[guinea::test(iterations = 1)]
fn a_scope_opens_one_feed_of_a_type_and_the_second_names_the_first(h: &mut Harness) {
    let twice = catch_unwind(AssertUnwindSafe(|| h.install::<twice::Twice>(&())));
    let refused = twice.err().map(said).unwrap_or_default();

    let first = format!("feeds.rs:{}", FIRST_OPENED_AT.get());
    assert!(refused.contains("Report") && refused.contains(&first), "{refused}");
}

#[guinea::test(iterations = 1)]
fn a_feed_argument_is_taken_from_a_segment_above_and_not_from_beside(h: &mut Harness) {
    h.install::<scanning::Scanning>(&()).unwrap();

    let refused = installed(h.install::<watching::Watching>(&())).unwrap_err();
    assert!(refused.contains("Feed<feeds::Report>"), "{refused}");
}

#[guinea::test(iterations = 1)]
fn within_a_segment_a_feed_is_handed_on_by_value(h: &mut Harness) {
    assert_eq!(installed(h.install::<beside::Beside>(&())), Ok(()));

    set(Report(2));
    assert_eq!(h.state::<Seen>().0, [1, 2]);
}

#[guinea::test(iterations = 1)]
fn an_optional_feed_nothing_above_opened_is_none(h: &mut Harness) {
    let alone = h.child();
    assert_eq!(installed(alone.install::<glancing::Glancing>(&())), Ok(()));
    assert_eq!(alone.state::<Seen>().0, [] as [u32; 0]);
    alone.leave();

    h.install::<scanning::Scanning>(&()).unwrap();
    let below = h.child();
    assert_eq!(installed(below.install::<glancing::Glancing>(&())), Ok(()));
    assert_eq!(below.state::<Seen>().0, [1]);
}

#[guinea::test(iterations = 1)]
fn what_is_missing_is_named_at_once_services_and_feeds_alike(h: &mut Harness) {
    let page = h.child();
    let refused = installed(page.install::<needing::Needing>(&())).unwrap_err();

    assert!(
        refused.contains("Needing")
            && refused.contains("Greeting")
            && refused.contains("Feed<feeds::Report>"),
        "{refused}"
    );
    assert!(!needing::RAN.get());
}

#[guinea::test(iterations = 1)]
fn writing_to_a_feed_whose_segment_is_gone_panics(h: &mut Harness) {
    let page = h.child();
    page.install::<scanning::Scanning>(&()).unwrap();
    page.leave();

    let written = catch_unwind(AssertUnwindSafe(|| set(Report(2))));
    let refused = written.err().map(said).unwrap_or_default();

    assert!(refused.contains("Report") && refused.contains("removed"), "{refused}");
}
