---
title: The event bus
sidebar:
  order: 7
---

Two actors sometimes need to communicate without being handed each other's
addresses. A connection controller can ask an agent to start, for example,
without knowing the concrete actor that owns the agent. They share a message
type, not an `Addr`.

The event bus connects them. A publisher sends a typed event; the actors
subscribed to that type receive it. Neither side needs a reference to the
other. There is one bus per application scope tree, shared by its windows,
layouts and pages. It is not a separate bus per scope and is not a
process-wide singleton.

## A use case from uniproc

In uniproc, `AgentLinkActor` handles a request to start the Windows agent
inside the application process. Once its checks permit that, it publishes
`RunWindowsAgentInProcess`. The Windows agent actor subscribes to that
message and switches its source to the in-process implementation.

The controller does not need the agent actor's address or its concrete
backend type. Both know the shared message contract. The following example
keeps that separation but uses the current guinea API and counters instead
of launching a real agent. It is not a copy of uniproc's integration code.

## The shared contract

<!-- shown: event bus: a shared contract -->
```rust
#[derive(Clone, Debug, Event)]
pub struct RunInProcess;

pub struct Start;
```
<!-- /shown -->

`Start` is an action sent to a particular feature through its dispatcher.
`RunInProcess` goes over the bus. `#[derive(Event)]` marks that shared
contract explicitly; bus events must be `Clone + Send + 'static`.

Put shared message types where both sides can depend on them. The publisher
need not depend on the subscriber's implementation, or vice versa.

## Publishing from an actor

<!-- shown: event bus: publishing from an actor -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Requests(pub u32);

#[reducer]
fn requests(this: &mut Requests, count: u32) {
    this.0 += count;
}

#[derive(Debug)]
struct Control {
    push: Push<Requests>,
}

actor! {
    Control {
        handlers { Start }
        publishes { RunInProcess }
    }
}

#[handler]
fn start(In(this, _, cx): In<Control, Start>) {
    this.push.send(1);
    cx.publish(RunInProcess);
}

feature! {
    pub AgentControl {
        exports { Requests }
    }
}

#[installs]
fn agent_control(cx: &Install) -> anyhow::Result<AgentControl> {
    let (requests, _) = cx.state::<Requests>().driven_by(|push| Control { push });
    Ok(AgentControl(requests))
}
```
<!-- /shown -->

The page emits `Start` through the dispatcher obtained with `Requests`.
`Control` records the request and publishes `RunInProcess` through its
context. `publishes` declares which event types that actor may publish.
The counter records an attempt, not proof that any agent has started.

## Subscribing an actor

<!-- shown: event bus: subscribing an actor -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Runs(pub u32);

#[reducer]
fn runs(this: &mut Runs, count: u32) {
    this.0 += count;
}

#[derive(Debug)]
struct Runtime {
    push: Push<Runs>,
}

actor! {
    Runtime {
        subscribes { RunInProcess }
    }
}

#[handler]
fn run_in_process(In(this, _, _): In<Runtime, RunInProcess>) {
    this.push.send(1);
}

feature! {
    pub AgentRuntime {
        exports { Runs }
    }
}

#[installs]
fn agent_runtime(cx: &Install) -> anyhow::Result<AgentRuntime> {
    let (runs, _) = cx.state::<Runs>().driven_by(|push| Runtime { push });
    Ok(AgentRuntime(runs))
}
```
<!-- /shown -->

`subscribes` connects the actor to the bus when it is created. The ordinary
`#[handler]` handles the incoming event. No manual call to subscribe and no
stored unsubscribe handle are needed.

The actors can live in sibling scopes, even though ordinary state lookup
does not cross between siblings:

<!-- shown: event bus: actors in sibling scopes communicate -->
```rust
#[guinea::test]
fn control_can_reach_a_runtime_without_its_address(h: &mut Harness) {
    let controls = h.child();
    let backend = h.child();
    controls.install::<AgentControl>(&()).unwrap();
    backend.install::<AgentRuntime>(&()).unwrap();

    controls.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(controls.state::<Requests>().0, 1);
    assert_eq!(backend.state::<Runs>().0, 1);
}
```
<!-- /shown -->

The bus does not make their state shared. Each actor still updates its own
feature's reducer. Only the message crosses the boundary.

## A broadcast, not a selected destination

Publication goes to every active subscriber of that event type in the same
tree. It does not search for the nearest actor or choose a single instance:

<!-- shown: event bus: every subscriber hears it -->
```rust
#[guinea::test]
fn publication_is_not_a_request_to_one_selected_actor(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    let first = h.child();
    let second = h.child();
    first.install::<AgentRuntime>(&()).unwrap();
    second.install::<AgentRuntime>(&()).unwrap();

    h.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(first.state::<Runs>().0, 1);
    assert_eq!(second.state::<Runs>().0, 1);
}
```
<!-- /shown -->

Here both runtimes are test counters. In an application, two subscriptions
to a command that must execute once would be a wiring mistake: the bus does
not elect one executor. Likewise, publishing with no subscribers is allowed
and produces no acknowledgement.

Address-free communication removes a concrete actor dependency, not the
message contract or the need to arrange the right listeners. For a result
from one answerer, use the bus's separate request/reply contract
(`Request`, `RpcRequest` and `request`) rather than treating `publish` as a
call that returns a result.

## Lifetime and missed events

An actor's bus subscriptions end when the actor is disposed. While its
scope sleeps, it receives no events. Waking does not replay what happened
during sleep:

<!-- shown: event bus: sleeping subscribers miss events -->
```rust
#[guinea::test]
fn waking_does_not_replay_events_published_during_sleep(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    let backend = h.child();
    backend.install::<AgentRuntime>(&()).unwrap();

    backend.scope().sleep();
    h.dispatch::<Requests>().emit(Start);
    h.settled();
    assert_eq!(backend.state::<Runs>().0, 0);

    backend.scope().wake();
    h.settled();
    assert_eq!(backend.state::<Runs>().0, 0);

    h.dispatch::<Requests>().emit(Start);
    h.settled();
    assert_eq!(backend.state::<Runs>().0, 1);
}
```
<!-- /shown -->

The event bus stores no current value or backlog. A subscriber created
after publication also misses that event. When a new or waking page needs
the latest snapshot instead, use a [feed](../feeds/).

This is also why a bus is not a durable job queue. If work must happen even
when a page leaves, place its consumer in a scope with the required lifetime
and choose an explicit completion and retry protocol where needed.

## A callback without an actor

A feature that only needs a small reaction can register a scoped callback:

<!-- shown: event bus: a scoped callback -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Notices(pub u32);

#[reducer]
fn notices(this: &mut Notices, count: u32) {
    this.0 += count;
}

feature! {
    pub Observing {
        exports { Notices }
    }
}

#[installs]
fn observing(cx: &Install) -> anyhow::Result<Observing> {
    let notices = cx.state::<Notices>().plain();
    cx.subscribe(move |_: RunInProcess| notices.push(1));
    Ok(Observing(notices))
}
```
<!-- /shown -->

`cx.subscribe` owns the subscription for the feature's scope. The callback
also receives nothing while that scope sleeps, and is removed with it.
Use this for a small reaction; use an actor when several handlers share
working state or need background tasks.

## Delivery and background work

Actors handle events on the UI thread. Keep those handlers short, just as
you would a handler of a page action; start slow work with
[`cx.task()`](../tasks/). A publication is not an instruction to run every
subscriber in a new background task.

`cx.async_ctx()` provides a context that can publish from background work.
Its `publish` schedules delivery on the UI thread and still checks the
actor's declared `publishes` contract. It does not turn the event bus into
a retained source of state.
