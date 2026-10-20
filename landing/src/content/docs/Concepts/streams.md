---
title: Streams and sources
sidebar:
  order: 9
---

A task returns one answer. A stream yields items over time: records from a
connection, file changes or process activity. `cx.task().pour(...)` connects
such a stream to an actor. guinea owns the running source in the actor's
scope, delivers its items on the UI thread and ends it when the actor leaves.

Unlike a [feed](../feeds/), this path does not replace one delivered item
with a newer snapshot. It is useful when the consumer needs each record
from a connection, rather than just its latest state.

The imports for the examples and their tests are:

<!-- shown: streams: imports -->
```rust
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use guinea::actor::Stream;
use guinea::app::Harness;
use guinea::prelude::*;
use tokio::sync::mpsc;
```
<!-- /shown -->

## Adapting a connection

The example uses a bounded Tokio channel instead of a network connection.
The producer sends records or a terminal error; dropping all senders means
EOF. A small `Stream` implementation translates those conditions into
messages the feature understands:

<!-- shown: streams: adapting a connection -->
```rust
#[derive(Clone, Debug)]
pub enum Incoming {
    Opened,
    Record(u32),
    Failed(String),
    Closed,
}

#[derive(Debug)]
pub struct Connection {
    receiver: mpsc::Receiver<Result<u32, String>>,
    ended: bool,
}

impl Stream for Connection {
    type Item = Incoming;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Incoming>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(record))) => Poll::Ready(Some(Incoming::Record(record))),
            Poll::Ready(Some(Err(error))) => {
                self.ended = true;
                Poll::Ready(Some(Incoming::Failed(error)))
            }
            Poll::Ready(None) => {
                self.ended = true;
                Poll::Ready(Some(Incoming::Closed))
            }
        }
    }
}

pub fn connection_pair() -> (mpsc::Sender<Result<u32, String>>, Connection) {
    let (sender, receiver) = mpsc::channel(2);
    (
        sender,
        Connection {
            receiver,
            ended: false,
        },
    )
}
```
<!-- /shown -->

`Stream` is re-exported as `guinea::actor::Stream`. A library's existing
stream can be used directly if it meets the bounds; the adapter here is
needed to define our protocol, not because guinea requires a custom type.
This example also uses Tokio's `sync` feature for the channel.

`Failed` is terminal by this adapter's choice. A generic stream item of
type `Result<T, E>` does not automatically end a source on `Err`.
Similarly, plain EOF ends a source without inventing an actor message.
Our adapter yields one `Closed` message before returning `None` so the
feature can record EOF. `Opened` is sent by the connection handler below.

## Consuming items in an actor

<!-- shown: streams: an actor consumes a stream -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Activity {
    pub records: Vec<u32>,
    pub error: Option<String>,
    pub closed: bool,
}

#[reducer]
fn activity(this: &mut Activity, incoming: Incoming) {
    match incoming {
        Incoming::Opened => {
            this.error = None;
            this.closed = false;
        }
        Incoming::Record(record) => this.records.push(record),
        Incoming::Failed(error) => {
            this.error = Some(error);
            this.closed = true;
        }
        Incoming::Closed => this.closed = true,
    }
}

pub struct Connect(pub Connection);
pub struct Disconnect;

#[derive(Debug)]
struct Collector {
    push: Push<Activity>,
}

actor! {
    Collector {
        handlers {
            Connect => { task Incoming }
            Incoming,
            Disconnect
        }
    }
}

#[handler]
fn connect(In(this, Connect(connection), cx): In<Collector, Connect>) {
    this.push.send(Incoming::Opened);
    cx.task().latest().pour(connection, std::convert::identity);
}

#[handler]
fn incoming(In(this, incoming, _): In<Collector, Incoming>) {
    this.push.send(incoming);
}

#[handler]
fn disconnect(In(this, _, cx): In<Collector, Disconnect>) {
    cx.task().lane::<Incoming>().cancel();
    this.push.send(Incoming::Closed);
}

feature! {
    pub Collecting {
        exports { Activity }
    }
}

#[installs]
fn collecting(cx: &Install) -> anyhow::Result<Collecting> {
    let (activity, _) = cx.state::<Activity>().driven_by(|push| Collector { push });
    Ok(Collecting(activity))
}
```
<!-- /shown -->

`Connect => { task Incoming }` declares that the handler can start work
which delivers `Incoming` messages. `pour` maps stream items into those
messages; `identity` is enough here because the types already match.
The stream must be `Send + 'static`, and the mapper and resulting messages
must also be `Send + 'static`.

`latest` applies to the **whole source**, not to each record. Another
`Connect` replaces the old source in the `Incoming` lane. The actor still
handles every accepted item from the selected connection.

This test feeds two records, then closes the input. Neither record is
replaced by the other:

<!-- shown: streams: items and eof -->
```rust
#[guinea::test]
fn a_connection_delivers_its_records_and_then_reports_eof(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    h.install::<Collecting>(&()).unwrap();
    h.dispatch::<Activity>().emit(Connect(connection));
    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Ok(2)).unwrap();
    drop(sender);
    h.settled();

    let activity = h.state::<Activity>();
    assert_eq!(activity.records, [1, 2]);
    assert!(activity.closed);
    assert_eq!(activity.error, None);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

The adapter's terminal error also reaches the reducer and closes the source:

<!-- shown: streams: a terminal error -->
```rust
#[guinea::test]
fn a_connection_error_is_a_terminal_message(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    h.install::<Collecting>(&()).unwrap();
    h.dispatch::<Activity>().emit(Connect(connection));
    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Err("connection lost".into())).unwrap();
    h.settled();

    assert_eq!(h.state::<Activity>().records, [1]);
    assert_eq!(
        h.state::<Activity>().error.as_deref(),
        Some("connection lost")
    );
    assert!(h.state::<Activity>().closed);
    assert!(sender.is_closed());
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

If a handler would only forward each item to a reducer, use
`cx.task().pushing(push).pour(stream, into)` instead. For a source that needs
no actor task policy, installation also offers `cx.pour(stream, receiver)`.

## Reconnecting and late items

Replacing a source cancels the old one and prevents further delivery from
it. Records already accepted into state remain; pending old records need
not be delivered. The replacement does not clear history automatically:

<!-- shown: streams: replacing the connection -->
```rust
#[guinea::test]
fn reconnecting_discards_the_old_connections_pending_items(h: &mut Harness) {
    let (old_sender, old_connection) = connection_pair();
    h.install::<Collecting>(&()).unwrap();
    let dispatch = h.dispatch::<Activity>();
    dispatch.emit(Connect(old_connection));
    old_sender.try_send(Ok(1)).unwrap();
    h.settled();

    old_sender.try_send(Ok(2)).unwrap();
    let (sender, connection) = connection_pair();
    dispatch.emit(Connect(connection));
    sender.try_send(Ok(3)).unwrap();
    h.settled();

    assert_eq!(h.state::<Activity>().records, [1, 3]);
    assert!(old_sender.is_closed());
    dispatch.emit(Disconnect);
    h.settled();
    assert!(sender.is_closed());
    assert!(h.state::<Activity>().closed);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

`Disconnect` cancels the named lane. The handler explicitly updates the
connection status: cancellation is not a natural EOF, so do not expect it
to manufacture the adapter's `Closed` item.

## Sleep, buffering and backpressure

A sleeping scope receives no source items. Pulling pauses, although a pull
already awaiting an item can finish and hold that item until wake. Items
already on their way to the UI thread are held for wake too. In this
example, the bounded input channel retains records while the page sleeps:

<!-- shown: streams: sleeping pauses consumption -->
```rust
#[guinea::test]
fn buffered_records_wait_until_the_page_wakes(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    let page = h.child();
    page.install::<Collecting>(&()).unwrap();
    page.dispatch::<Activity>().emit(Connect(connection));
    h.settled();
    page.scope().sleep();

    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Ok(2)).unwrap();
    h.settled();
    assert!(page.state::<Activity>().records.is_empty());

    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Activity>().records, [1, 2]);

    page.leave();
    h.settled();
    assert!(sender.is_closed());
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Both records arrive after wake, unlike a feed that catches up with only
its last snapshot. Removing the page instead cancels the source and drops
the receiver; it does not retain a queue for the next page instance.

Channel capacity and overflow are part of the source's contract. Here
`try_send` reports `Full` instead of silently losing data:

<!-- shown: streams: bounded input -->
```rust
#[guinea::test]
fn a_bounded_connection_exposes_backpressure(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Ok(2)).unwrap();
    assert!(matches!(
        sender.try_send(Ok(3)),
        Err(mpsc::error::TrySendError::Full(_))
    ));

    h.install::<Collecting>(&()).unwrap();
    h.dispatch::<Activity>().emit(Connect(connection));
    h.settled();
    sender.try_send(Ok(3)).unwrap();
    h.settled();
    assert_eq!(h.state::<Activity>().records, [1, 2, 3]);
}
```
<!-- /shown -->

An asynchronous producer can use the channel's `send(...).await` to wait for
capacity, or choose an explicit overflow policy. The channel's capacity is
not a bound on the entire pipeline: items already pulled or queued for UI
delivery are outside it. guinea does not provide durable storage or a
global backpressure guarantee for an arbitrary external producer.

For an activity log that must survive disconnection or navigation, put
collection in a sufficiently long-lived scope. Replay from history and
merge it with live items using the producer protocol's cursor or sequence
number. `pour` does not invent replay, deduplication or exactly-once
delivery, and a coalescing feed is not a substitute for that protocol.

## Timers are sources too

`repeat` opens a source of ticks owned by its installation scope:

<!-- shown: streams: a timer is a source -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Pulses(pub u32);

#[reducer]
fn pulses(this: &mut Pulses, _: ()) {
    this.0 += 1;
}

feature! {
    pub Polling {
        exports { Pulses }
    }
}

#[installs]
fn polling(cx: &Install) -> anyhow::Result<Polling> {
    let pulses = cx.state::<Pulses>().plain();
    cx.repeat(Duration::from_secs(1), move || pulses.push(()))
        .named("poll");
    Ok(Polling(pulses))
}
```
<!-- /shown -->

The first tick waits for the period; it is not an immediate callback.
Sleeping does not replay every elapsed interval on wake:

<!-- shown: streams: timer sleep and wake -->
```rust
#[guinea::test]
fn a_timer_waits_for_its_first_period_and_does_not_replay_every_missed_tick(h: &mut Harness) {
    let page = h.child();
    page.install::<Polling>(&()).unwrap();
    h.settled();
    assert_eq!(page.state::<Pulses>().0, 0);
    h.advance(Duration::from_secs(1));
    assert_eq!(page.state::<Pulses>().0, 1);

    page.scope().sleep();
    h.advance(Duration::from_secs(5));
    assert_eq!(page.state::<Pulses>().0, 1);
    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Pulses>().0, 2);
    h.advance(Duration::from_secs(1));
    assert_eq!(page.state::<Pulses>().0, 3);

    page.leave();
    h.settled();
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

`every(period, message, receiver)` sends a freshly constructed message per
tick instead of running a callback. `Ticks::every(period)` exposes the
underlying stream for composition. Fixed periods accept `Duration`.

These APIs return a `Source` handle where applicable. The handle can name
the source for tools with `.named(...)`; dropping it does not stop work,
and keeping it does not extend the scope's lifetime. The scope owns the
source. To stop an actor's task source early, use its lane's `cancel`, as
`Disconnect` does above.
