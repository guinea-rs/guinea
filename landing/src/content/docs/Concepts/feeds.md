---
title: Feeds
sidebar:
  order: 6
---

A background producer may have a value that several features need: the
current temperature, a process snapshot or a connection's status. Consumers
need what is true **now**, including when they first appear or wake after
being kept asleep. They do not necessarily need every intermediate change.

`Feed<T>` carries that latest snapshot. The feature or plugin that opens it
gets a `FeedWriter<T>`; consumers get a read-only `Feed<T>`. The producer
chooses who receives a clone of the writer. A consumer can use the feed
without knowing which actor, task or external library supplies its values.

## Opening a feed

This small plugin opens a feed of temperature readings. It does not connect
to a sensor: it returns the writer so the application can hand it to a
producer, and so our tests can supply readings directly.

<!-- shown: feeds: opening a feed -->
```rust
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
```
<!-- /shown -->

`app.feed::<Reading>()` opens the feed in the application's scope.
`feeds { Reading }` declares the returned read handle; `provides` adds the
writer to the plugin's returned value. As with other plugin services,
returning the writer does not call `app.provide` automatically.

A feature opens a feed in its own scope with `cx.feed::<T>()` in the same
way. A scope can open only one feed of each type; a second attempt panics.
Different scopes can open feeds of the same type independently.

`T` must be `Send + Sync + 'static`. `set` accepts either a `T` or an
`Arc<T>`, and the feed retains the latest value. There is no initial value
until the first `set`.

## Receiving snapshots in a feature

An installation function can request `Feed<Reading>` as an argument:

<!-- shown: feeds: following from a feature -->
```rust
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
```
<!-- /shown -->

The injected feed comes from the nearest **ancestor** scope that opened one.
Lookup does not search siblings or the installation scope itself. If
another feature in the same scope opens the feed, pass its returned handle
explicitly, for example through installation parameters.

`cx.follow` registers a callback for this consumer's scope. An awake consumer
receives the current value immediately, if one exists, and later receives
new snapshots while it remains awake. The callback gets an `Arc<Reading>`
on the UI thread; it can send an update through a `Bound` or `Push` handle.
No unsubscribe handle needs to be stored: removing the consumer's scope
ends the subscription.

Here `Seen` records the readings delivered to the feature, so the tests can
show their order. In an application, the reducer could instead replace a
single displayed temperature. The feed itself stores no such history.

<!-- shown: feeds: the current value reaches a new follower -->
```rust
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
```
<!-- /shown -->

## The producer: writing new snapshots

The previous test writes through the returned handle directly. In an
application, that call belongs to the producer. Here is an alternative
thermometer plugin that keeps the writer inside a timer callback and returns
only the read handle:

<!-- shown: feeds: a producer writes readings -->
```rust
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
```
<!-- /shown -->

Every second the callback calls `writer.set(...)`. The increasing numbers
stand in for sensor readings: this example produces `20`, `21`, and so on,
without contacting a real device. A sensor callback or an actor can publish
its measured value at the same point.

`PollingThermometer` replaces `Thermometer` in application setup; do not
install both in the same scope, since both open a feed of `Reading`.
`Watching` needs no changes. This test exercises the whole path without
writing into the feed itself:

<!-- shown: feeds: produced readings reach a consumer -->
```rust
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
```
<!-- /shown -->

The callback owns the writer, and the application's scope owns the timer.
Dropping the application stops production as well as removing the feed.
An external source would need its own scoped cleanup instead.

## Reading without subscribing

For a one-off read, `feed.get()` returns `Option<Arc<T>>` without subscribing.
`None` means either that no value has been set or that the producer's scope
has been removed. `Option<Feed<T>>` as an installation argument makes the
dependency optional; that is different from an existing feed with no value.

## Sleep and wake

A kept page may sleep without being removed. Its subscriptions remain
registered, but it receives no snapshots while asleep. If the feed changes,
waking delivers only the latest value, once:

<!-- shown: feeds: waking catches up once -->
```rust
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
```
<!-- /shown -->

The intermediate reading `21` is not replayed. Sleeping and waking again
without another write produces no duplicate delivery. The test uses
`scope().sleep()` and `wake()` to exercise this directly; a mounted page's
lifecycle is normally managed by the router.

## A snapshot is not an event queue

Several writes in one UI turn can become one notification with the last
value. Writes from another thread are also coalesced: a burst schedules one
hop to the UI thread, where consumers receive the latest available value.
Do not use a feed when every item must be processed or replayed.

The feed does not compare values using `PartialEq`. Setting the same `Arc`
allocation again is ignored, but a newly allocated snapshot with equal
contents is still a new write. Retaining an `Arc` does not make mutating
its contents a feed update: publish a new snapshot with `set`.

Choose the mechanism by the contract you need:

- **Reducer state** holds a feature's model and changes through its updates.
  A page can read it and obtain the owning feature's dispatcher.
- **A feed** offers the producer's latest snapshot to other consumers, with
  scoped subscriptions and catch-up after sleep. It grants no dispatcher or
  write access to the producer.
- **A bus event** announces something that happened. It has no retained
  current value for a consumer that appears later.

A feed can be transformed into local reducer state, as `Watching` does.
That does not make the consumer the owner of the original data.

## Producing only when someone is watching

The section that opens a feed can answer `Followed<T>`. It receives `true`
when the first awake subscriber appears and `false` when the last one
leaves or sleeps. This lets a producer decide whether polling is needed.

The following plugin only records those transitions; it does not start
or stop a real sensor:

<!-- shown: feeds: noticing demand -->
```rust
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
```
<!-- /shown -->

<!-- shown: feeds: only awake followers count -->
```rust
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
```
<!-- /shown -->

Two awake consumers still mean one `true`, not two start requests. Removing
one does not end demand while another remains awake. Waking the remaining
consumer starts demand again. `Followed` is delivered to the opening
section, not broadcast as a bus event. An actor in that section can handle
it too, when registered as the section's answerer.

The signal does not stop work by itself. Its handler must start or cancel
the producer's work, using the mechanisms from [background tasks](../tasks/)
or the external library's own API.

## Ownership and other threads

`Feed` is `Copy`; copying it does not keep its scope alive. A snapshot already
returned as an `Arc` is owned separately and can outlive that scope:

<!-- shown: feeds: handles do not keep the scope alive -->
```rust
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
```
<!-- /shown -->

`FeedWriter` is cloneable and can be moved into a background task or thread.
Its clones share one feed; they do not create independent producers or
extend the scope's lifetime. Once the producer's scope is removed, writing
on the UI thread panics. A late write from another thread is dropped with
a warning instead.

Removing a feed does not stop arbitrary external threads. Tie their
shutdown to the scope as well, using task cancellation or
[`on_teardown`](../scopes/#keeping-an-external-resource-until-the-page-leaves).
