use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use guinea::app::{Harness, TestApp};
use guinea::prelude::*;

//@show feeds: opening a feed
#[derive(Debug)]
pub struct Reading(pub i32);

plugin! {
    pub Thermometer {
        feeds { Reading }
        provides { FeedWriter<Reading> }
    }
}

#[builds]
fn thermometer(app: &mut PluginBuilder) -> anyhow::Result<Thermometer> {
    let writer = app.feed::<Reading>();
    Ok(Thermometer(writer.feed(), writer))
}
//@show-end

//@show feeds: following from a feature
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Seen(pub Vec<i32>);

#[reducer]
fn seen(this: &mut Seen, temperature: i32) {
    this.0.push(temperature);
}

feature! {
    pub Watching {
        exports { Seen }
    }
}

#[installs]
fn watching(cx: &Install, readings: Feed<Reading>) -> anyhow::Result<Watching> {
    let seen = cx.state::<Seen>().plain();
    cx.follow(readings, move |reading| seen.push(reading.0));
    Ok(Watching(seen))
}
//@show-end

//@show feeds: a producer writes readings
plugin! {
    pub PollingThermometer {
        feeds { Reading }
    }
}

#[builds]
fn polling_thermometer(app: &mut PluginBuilder) -> anyhow::Result<PollingThermometer> {
    let writer = app.feed::<Reading>();
    let readings = writer.feed();
    let mut temperature = 20;

    app.repeat(Duration::from_secs(1), move || {
        writer.set(Reading(temperature));
        temperature += 1;
    });

    Ok(PollingThermometer(readings))
}
//@show-end

//@show feeds: produced readings reach a consumer
#[guinea::test]
fn the_producer_writes_and_the_consumer_receives(h: &mut Harness) {
    let PollingThermometer(readings) = h.plugin::<PollingThermometer>(()).unwrap();
    h.install::<Watching>(&()).unwrap();
    assert!(readings.get().is_none());
    assert!(h.state::<Seen>().0.is_empty());

    h.advance(Duration::from_secs(1));
    assert_eq!(readings.get().unwrap().0, 20);
    assert_eq!(h.state::<Seen>().0, [20]);

    h.advance(Duration::from_secs(1));
    assert_eq!(readings.get().unwrap().0, 21);
    assert_eq!(h.state::<Seen>().0, [20, 21]);
}
//@show-end

#[guinea::test]
fn dropping_the_application_stops_the_feed_producer(h: &mut Harness) {
    let mut app = TestApp::new();
    let PollingThermometer(readings) = app.plugin::<PollingThermometer>(()).unwrap();
    h.advance(Duration::from_secs(2));
    assert_eq!(readings.get().unwrap().0, 21);

    drop(app);
    h.advance(Duration::from_secs(2));
    assert!(readings.get().is_none());
    assert_eq!(h.stuck(), 0);
}

//@show feeds: the current value reaches a new follower
#[guinea::test]
fn a_new_follower_receives_the_current_reading(h: &mut Harness) {
    let Thermometer(readings, writer) = h.plugin::<Thermometer>(()).unwrap();
    assert!(readings.get().is_none());

    writer.set(Reading(20));
    h.install::<Watching>(&()).unwrap();
    assert_eq!(h.state::<Seen>().0, [20]);

    writer.set(Reading(21));
    assert_eq!(readings.get().unwrap().0, 21);
    assert_eq!(h.state::<Seen>().0, [20, 21]);
}
//@show-end

//@show feeds: waking catches up once
#[guinea::test]
fn a_sleeping_page_catches_up_with_the_latest_reading(h: &mut Harness) {
    let Thermometer(_, writer) = h.plugin::<Thermometer>(()).unwrap();
    writer.set(Reading(20));
    let page = h.child();
    page.install::<Watching>(&()).unwrap();

    page.scope().sleep();
    writer.set(Reading(21));
    writer.set(Reading(22));
    assert_eq!(page.state::<Seen>().0, [20]);

    page.scope().wake();
    assert_eq!(page.state::<Seen>().0, [20, 22]);

    page.scope().sleep();
    page.scope().wake();
    assert_eq!(page.state::<Seen>().0, [20, 22]);
}
//@show-end

//@show feeds: noticing demand
plugin! {
    pub Demand {
        feeds { Reading }
    }
}

#[builds]
fn demand(app: &mut PluginBuilder, changes: Rc<RefCell<Vec<bool>>>) -> anyhow::Result<Demand> {
    app.answers(move |followed: Followed<Reading>| changes.borrow_mut().push(followed.0));
    let writer = app.feed::<Reading>();
    Ok(Demand(writer.feed()))
}
//@show-end

//@show feeds: only awake followers count
#[guinea::test]
fn demand_ends_only_when_the_last_awake_follower_goes(h: &mut Harness) {
    let changes = Rc::new(RefCell::new(Vec::new()));
    h.plugin::<Demand>(changes.clone()).unwrap();

    let first = h.child();
    let second = h.child();
    first.install::<Watching>(&()).unwrap();
    second.install::<Watching>(&()).unwrap();
    assert_eq!(*changes.borrow(), [true]);

    first.scope().sleep();
    assert_eq!(*changes.borrow(), [true]);
    second.leave();
    assert_eq!(*changes.borrow(), [true, false]);

    first.scope().wake();
    assert_eq!(*changes.borrow(), [true, false, true]);
    first.leave();
    assert_eq!(*changes.borrow(), [true, false, true, false]);
}
//@show-end

#[guinea::test]
fn reusing_an_arc_is_not_a_new_reading(h: &mut Harness) {
    let Thermometer(_, writer) = h.plugin::<Thermometer>(()).unwrap();
    h.install::<Watching>(&()).unwrap();
    let reading = Arc::new(Reading(20));
    writer.set(reading.clone());
    writer.set(reading);
    assert_eq!(h.state::<Seen>().0, [20]);

    writer.set(Reading(20));
    assert_eq!(h.state::<Seen>().0, [20, 20]);
}

//@show feeds: handles do not keep the scope alive
#[test]
fn a_feed_handle_does_not_keep_its_owner_alive() {
    let mut app = TestApp::new();
    let Thermometer(readings, writer) = app.plugin::<Thermometer>(()).unwrap();
    writer.set(Reading(20));
    let snapshot = readings.get().unwrap();

    drop(app);

    assert!(readings.get().is_none());
    assert_eq!(snapshot.0, 20);
}
//@show-end
