//@show rpc: imports
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use guinea::actor::event_bus::{RpcRequest, Why};
use guinea::app::Harness;
use guinea::prelude::*;
//@show-end

//@show rpc: request and response
#[derive(Clone, Debug, guinea::Request)]
#[request(reply = Outcome)]
pub struct Terminate(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done,
    Denied,
}
//@show-end

//@show rpc: an answerer
#[derive(Clone, Debug, Default)]
pub struct ServerParams {
    pub delay: Duration,
    pub completed: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct Processes(ServerParams);

actor! {
    Processes {
        subscribes { RpcRequest<Terminate> }
    }
}

#[handler]
fn terminate(In(this, Terminate(pid), _): In<Processes, Terminate>) -> Reply<Outcome> {
    let params = this.0.clone();
    Reply::later(async move {
        tokio::time::sleep(params.delay).await;
        params.completed.fetch_add(1, Ordering::SeqCst);
        match pid {
            4 => Outcome::Denied,
            _ => Outcome::Done,
        }
    })
}

feature! {
    pub ProcessService {}
}

#[installs]
fn process_service(cx: &Install<ServerParams>) -> anyhow::Result<ProcessService> {
    cx.spawn(Processes(cx.params().clone()));
    Ok(ProcessService)
}
//@show-end

//@show rpc: a client
#[derive(Default, Clone, PartialEq, Debug)]
pub struct LastReply(pub Option<Result<Outcome, Why>>);

#[derive(Clone)]
pub struct Answered(pub Result<Outcome, Why>);

#[reducer]
fn last_reply(this: &mut LastReply, answer: Answered) {
    this.0 = Some(answer.0);
}

pub struct Ask(pub u32);
pub struct CancelRequest;

#[derive(Debug)]
struct Client {
    push: Push<LastReply>,
}

actor! {
    Client {
        handlers { Ask, CancelRequest }
    }
}

#[handler]
fn ask(In(this, Ask(pid), cx): In<Client, Ask>) {
    let remote = cx.async_ctx();
    cx.task().latest().pushing(this.push).spawn(async move {
        let answer = remote.request(Terminate(pid), Duration::from_secs(2)).await;
        Answered(answer.map_err(|error| error.why))
    });
}

#[handler]
fn cancel_request(In(_, _, cx): In<Client, CancelRequest>) {
    cx.task().lane::<Answered>().cancel();
}

feature! {
    pub Requesting {
        exports { LastReply }
    }
}

#[installs]
fn requesting(cx: &Install) -> anyhow::Result<Requesting> {
    let (reply, _) = cx.state::<LastReply>().driven_by(|push| Client { push });
    Ok(Requesting(reply))
}
//@show-end

//@show rpc: an answer without an address
#[guinea::test]
fn a_client_receives_the_typed_answer(h: &mut Harness) {
    let server = h.child();
    let client = h.child();
    server
        .install::<ProcessService>(&ServerParams::default())
        .unwrap();
    client.install::<Requesting>(&()).unwrap();

    client.dispatch::<LastReply>().emit(Ask(42));
    h.settled();
    assert_eq!(client.state::<LastReply>().0, Some(Ok(Outcome::Done)));

    client.dispatch::<LastReply>().emit(Ask(4));
    h.settled();
    assert_eq!(client.state::<LastReply>().0, Some(Ok(Outcome::Denied)));
}
//@show-end

//@show rpc: a fake answerer
#[guinea::test]
fn a_test_can_replace_the_service_with_a_callback(h: &mut Harness) {
    let _answerer = h.bus().answer_fn(|_: Terminate| Outcome::Denied);
    h.install::<Requesting>(&()).unwrap();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();

    assert_eq!(h.state::<LastReply>().0, Some(Ok(Outcome::Denied)));
}
//@show-end

#[guinea::test]
fn a_request_without_an_answerer_fails_without_waiting_for_its_timeout(h: &mut Harness) {
    h.install::<Requesting>(&()).unwrap();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();
    assert_eq!(h.state::<LastReply>().0, Some(Err(Why::Nobody)));
}

#[guinea::test]
fn a_sleeping_answerer_is_not_a_queue_for_later(h: &mut Harness) {
    let server = h.child();
    server
        .install::<ProcessService>(&ServerParams::default())
        .unwrap();
    h.install::<Requesting>(&()).unwrap();
    server.scope().sleep();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();
    assert!(matches!(
        h.state::<LastReply>().0,
        Some(Err(Why::Asleep { .. }))
    ));

    server.scope().wake();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();
    assert_eq!(h.state::<LastReply>().0, Some(Ok(Outcome::Done)));
}

//@show rpc: timeout does not cancel the operation
#[guinea::test]
fn a_timeout_ends_waiting_but_the_service_can_finish_later(h: &mut Harness) {
    let params = ServerParams {
        delay: Duration::from_secs(5),
        ..Default::default()
    };
    h.install::<ProcessService>(&params).unwrap();
    h.install::<Requesting>(&()).unwrap();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();

    h.advance(Duration::from_secs(2));
    assert_eq!(h.state::<LastReply>().0, Some(Err(Why::TimedOut)));
    assert_eq!(params.completed.load(Ordering::SeqCst), 0);

    h.advance(Duration::from_secs(3));
    assert_eq!(params.completed.load(Ordering::SeqCst), 1);
    assert_eq!(h.state::<LastReply>().0, Some(Err(Why::TimedOut)));
}
//@show-end

//@show rpc: cancelling the caller is not remote cancellation
#[guinea::test]
fn cancelling_the_client_does_not_cancel_an_accepted_request(h: &mut Harness) {
    let params = ServerParams {
        delay: Duration::from_secs(5),
        ..Default::default()
    };
    h.install::<ProcessService>(&params).unwrap();
    h.install::<Requesting>(&()).unwrap();
    let dispatch = h.dispatch::<LastReply>();
    dispatch.emit(Ask(42));
    h.settled();

    dispatch.emit(CancelRequest);
    h.advance(Duration::from_secs(5));

    assert_eq!(params.completed.load(Ordering::SeqCst), 1);
    assert_eq!(h.state::<LastReply>().0, None);
    assert_eq!(h.stuck(), 0);
}
//@show-end

#[test]
#[should_panic(expected = "a request has exactly one answerer")]
fn a_request_type_cannot_have_two_answerers() {
    let h = Harness::new(0);
    h.install::<ProcessService>(&ServerParams::default())
        .unwrap();
    h.child()
        .install::<ProcessService>(&ServerParams::default())
        .unwrap();
}
