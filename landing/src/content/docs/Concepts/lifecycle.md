---
title: Sleep, wake and background work
sidebar:
  order: 10
---

A kept page is still installed. Sleeping is not removal, and it is not a
global pause of everything in its scope. Each communication mechanism has
its own rule:

| Resource | While asleep | After wake | On removal |
| --- | --- | --- | --- |
| Page/layout fields, actor fields, reducer state | Retained | Same instances | Released with the scope |
| Bus actor subscriptions and scoped callbacks | Miss events | No replay | Unsubscribed |
| Feed follower | No callbacks | Latest changed snapshot once | Unsubscribed |
| Stream source | Pauses pulling/delivery; an in-flight pull can finish | Delivers held items in order and resumes | Cancelled |
| Timer source | No callback | At most one overdue tick, then the next period | Cancelled |
| Ordinary task, including `latest`/`conflate` | Keeps running; replies can change state | No special replay needed | Cancelled; pending results discarded |
| Cooperative `spawn_with` task | Keeps running | No special replay | Token signalled; cleanup can outlive scope |

`keep()` preserves page-local fields as well as feature state. Removing a
parent removes its subtree. A parent's sleep alone is not a public promise
that arbitrary manually created child scopes are recursively asleep; use
the mounted tree's lifecycle rather than managing only one ancestor gate.

## An ordinary answer can arrive during sleep

The following feature starts an ordinary task with an actor reply, another
with `pushing`, and follows a feed. Its wake hooks surround that subscription:

<!-- shown: lifecycle: ordinary work and wake hooks -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Log(pub Vec<String>);

#[reducer]
fn log(this: &mut Log, item: String) {
    this.0.push(item);
}

pub struct Begin;
pub struct Answered;
pub struct Queue;
pub struct Direct;

#[derive(Debug)]
struct Worker {
    push: Push<Log>,
}

actor! {
    Worker {
        handlers {
            Begin => { task Answered }
            Answered,
            Queue => { send Answered }
            Direct
        }
    }
}

#[handler]
fn begin(In(_, _, cx): In<Worker, Begin>) {
    cx.task().latest().spawn(async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Answered
    });
}

#[handler]
fn answered(In(this, _, _): In<Worker, Answered>) {
    this.push.send("answer".into());
}

#[handler]
fn queue(In(_, _, cx): In<Worker, Queue>) {
    cx.send(Answered);
}

#[handler]
fn direct(In(this, _, cx): In<Worker, Direct>) {
    cx.task().pushing(this.push).spawn(async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        "direct".to_owned()
    });
}

feature! {
    pub Working {
        exports { Log }
    }
}

#[installs]
fn working(cx: &Install<Feed<u32>>) -> anyhow::Result<Working> {
    let (log, _) = cx.state::<Log>().driven_by(|push| Worker { push });
    cx.on_wake(move || log.push("before feed".into()));
    cx.follow(*cx.params(), move |value| log.push(format!("feed {value}")));
    cx.on_wake(move || log.push("after feed".into()));
    Ok(Working(log))
}
```
<!-- /shown -->

The imports are `guinea::prelude::*`, `guinea::app::Harness` and
`std::time::Duration`; the simulated work uses Tokio's `time` feature.
This test opens a feed in the harness's parent scope, then sleeps the child
while its tasks are waiting:

<!-- shown: lifecycle: a task can answer during sleep -->
```rust
#[guinea::test]
fn sleep_does_not_pause_ordinary_tasks_or_their_answers(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    writer.set(1);
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    page.dispatch::<Log>().emit(Begin);
    page.dispatch::<Log>().emit(Direct);
    h.settled();

    page.scope().sleep();
    h.advance(Duration::from_secs(1));
    let log = page.state::<Log>();
    assert_eq!(log.0.len(), 3);
    assert!(log.0.contains(&"answer".into()));
    assert!(log.0.contains(&"direct".into()));
}
```
<!-- /shown -->

Both results reach state while the page is asleep. Their relative order is
not guaranteed. Ordinary task replies are not bus events and are not stream
items. Direct address sends and feature actions are not gated by sleep
either. An actor's self-messages queued while it is busy are drained after
its current handler; sleep does not convert that queue into a wake backlog.
Disposal, unlike sleep, stops further queued handling.

`latest` still rejects superseded answers, and `conflate` can finish its
current run and start its newest pending work during sleep. Neither policy
means “wait until the page is visible.” If visibility should stop work,
define that explicitly with sleep/wake hooks and cancellation/restart logic.

The superseded run is rejected even if both requests happen across sleep:

<!-- shown: lifecycle: latest while asleep -->
```rust
#[guinea::test]
fn latest_still_rejects_the_old_answer_while_the_page_sleeps(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    let dispatch = page.dispatch::<Log>();
    dispatch.emit(Begin);
    h.settled();
    page.scope().sleep();
    h.advance(Duration::from_millis(500));
    dispatch.emit(Begin);
    h.settled();
    h.advance(Duration::from_millis(500));
    assert!(page.state::<Log>().0.is_empty());
    h.advance(Duration::from_millis(500));
    assert_eq!(page.state::<Log>().0, ["answer"]);
}
```
<!-- /shown -->

## Wake ordering

Wake marks the scope awake and invokes its hooks in registration order.
`follow` registers a hook too. There is no universal rule saying all user
hooks run before or after feed catch-up:

<!-- shown: lifecycle: wake hooks follow registration order -->
```rust
#[guinea::test]
fn feed_catchup_runs_between_the_hooks_registered_around_follow(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    writer.set(1);
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    page.scope().sleep();
    writer.set(2);
    assert_eq!(page.state::<Log>().0, ["feed 1"]);

    page.scope().wake();
    assert_eq!(
        page.state::<Log>().0,
        ["feed 1", "before feed", "feed 2", "after feed"]
    );
}
```
<!-- /shown -->

Here the hook registered before `follow` sees the old state; the feed
callback then updates it, and the later hook runs after that callback.
This ordering applies to these synchronous hooks, not to every background
or UI-queued consequence. Source delivery resumes asynchronously. If several
inputs must be reconciled together, coordinate them explicitly instead of
assuming wake is a single globally settled snapshot.

## Choosing a lifetime

Use `keep` for state that should survive a temporary route change. Put
collection that must continue across page removal in a layout or application
scope. Do not use a sleeping page as a durable event collector: bus events
are missed, and external source buffers can fill or overflow.

Removal cancels ordinary work whether the page was awake or asleep.
Cooperative cleanup is different: see the channel-based example and tests
in [background tasks](../tasks/#finishing-asynchronous-cleanup).
