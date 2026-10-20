---
title: Requests and replies
sidebar:
  order: 8
---

An [event](../event-bus/) tells subscribers something without returning a
result. When a caller needs an answer from one service without knowing its
actor's address, use a typed request. This RPC is between parts of the same
application, over its bus; it is not a network transport.

The examples use these imports, including the harness for their tests:

<!-- shown: rpc: imports -->
```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use guinea::actor::event_bus::{RpcRequest, Why};
use guinea::app::Harness;
use guinea::prelude::*;
```
<!-- /shown -->

## The contract

A request names its response type:

<!-- shown: rpc: request and response -->
```rust
#[derive(Clone, Debug, guinea::Request)]
#[request(reply = Outcome)]
pub struct Terminate(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done,
    Denied,
}
```
<!-- /shown -->

`Outcome::Denied` is an ordinary answer from the service. It differs from
failing to obtain an answer at all. The request and response types must be
`Clone + Send + 'static`.

The example simulates process termination. It never calls an OS process
API: PID `4` is denied and other IDs return `Done`.

## One service answers

<!-- shown: rpc: an answerer -->
```rust
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
```
<!-- /shown -->

The actor subscribes to `RpcRequest<Terminate>`, but its typed handler takes
`In<Processes, Terminate>`. Returning `Reply<Outcome>` makes it an answerer;
the generated implementation sends that answer to the caller.

`Reply::later` runs the future in the background. Here a configured delay
simulates the operation, and an atomic counter records its completion for
tests. The actor's mutable borrow ends when the handler returns: clone the
inputs needed by the future, not a reference to the actor.

For an immediate answer, return `Outcome` directly from the handler, or
use `Reply::now(outcome)`. Do not await slow work on the UI thread.

There can be only one answerer per request type on a bus. Registering a
second one panics, even if it is in another scope or the first one sleeps.
Other subscribers may observe the request without answering; an ordinary
handler returning no value is a listener, not another answerer.

## A client waits off the UI thread

<!-- shown: rpc: a client -->
```rust
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
```
<!-- /shown -->

The page emits `Ask` through its feature's dispatcher. The handler takes an
`AsyncContext` from its context, starts a task and awaits `request` there. The response is
either `Outcome` or an `Unanswered` error. This example keeps the error's
`why` in state; the full error also identifies the request type.

`pushing` applies the result in the UI thread. `latest` prevents an older
client task from replacing a newer task's result. The default lane is the
task's output type, `Answered`, not the request type `Terminate`.

The service and client can be installed in sibling scopes:

<!-- shown: rpc: an answer without an address -->
```rust
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
```
<!-- /shown -->

## Replacing the service in a test

`answer_fn` supplies a synchronous answer without constructing an actor:

<!-- shown: rpc: a fake answerer -->
```rust
#[guinea::test]
fn a_test_can_replace_the_service_with_a_callback(h: &mut Harness) {
    let _answerer = h.bus().answer_fn(|_: Terminate| Outcome::Denied);
    h.install::<Requesting>(&()).unwrap();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();

    assert_eq!(h.state::<LastReply>().0, Some(Ok(Outcome::Denied)));
}
```
<!-- /shown -->

Keep its returned subscription guard alive for the test; dropping it removes
the answerer. It takes the same unique answerer slot, so do not install the
real service alongside the fake. Unlike `cx.subscribe`, this low-level
callback has no consumer scope that automatically puts it to sleep.

## When there is no answer

`request` returns `Unanswered` with one of these reasons:

| `Why` | Meaning |
| --- | --- |
| `Nobody` | No subscriber is registered as the answerer. |
| `Asleep { answerer }` | The registered answerer's scope is asleep. |
| `Dropped` | The ability to reply was dropped without an answer. |
| `TimedOut` | The caller's deadline elapsed. |
| `TreeGone` | The application tree was gone when delivery was attempted. |

`Nobody` and `Asleep` fail without waiting for the deadline. A sleeping
service is not a queue: waking it does not replay that failed request.
The caller must decide whether and when to retry.

## A timeout is not cancellation of the operation

The client above waits for two seconds. Give the service a five-second
operation and observe both sides:

<!-- shown: rpc: timeout does not cancel the operation -->
```rust
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
```
<!-- /shown -->

The caller has stopped waiting, but the service still finishes. Its late
reply does not replace `TimedOut` in the client's state. In a real operation,
the side effect may have happened even though the caller saw no answer.
Automatic retry therefore needs an application-level idempotency policy.

## Cancelling the client

Cancelling the task that awaits `request` also does not retract a request
already delivered to the service:

<!-- shown: rpc: cancelling the caller is not remote cancellation -->
```rust
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
```
<!-- /shown -->

The service's completion counter advances, but the cancelled client's
reducer receives no result. Replacing the client task with `latest`, or
removing its page, has the same limitation: it stops that caller's work,
not an accepted operation on another actor.

An already-started `Reply::later` future is not automatically dropped when
its answerer actor leaves either. If the operation must support cancellation,
make that an explicit service protocol, with an operation ID and a cancel
request or token the service actually observes. Do not infer cancellation
from the absence of a waiting client.
