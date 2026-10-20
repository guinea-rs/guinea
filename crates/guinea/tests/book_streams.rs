//@show streams: imports
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use guinea::actor::Stream;
use guinea::app::Harness;
use guinea::prelude::*;
use tokio::sync::mpsc;
//@show-end

//@show streams: adapting a connection
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
//@show-end

//@show streams: an actor consumes a stream
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
//@show-end

//@show streams: items and eof
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
//@show-end

//@show streams: a terminal error
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
//@show-end

//@show streams: replacing the connection
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
//@show-end

//@show streams: sleeping pauses consumption
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
//@show-end

//@show streams: bounded input
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
//@show-end

//@show streams: a timer is a source
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
//@show-end

//@show streams: timer sleep and wake
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
//@show-end
