---
title: 'Background tasks: state and execution order'
sidebar:
  order: 11
---

## When the input snapshot is taken

`spawn(future)` receives a future built at the call site. Expressions used
to construct that future run now; the body of an `async` block runs only
when polled. A busy `conflate` lane keeps only its newest waiting request.

`spawn_with(factory)` can defer constructing that future too. Its factory
runs on the UI thread when the lane admits the work, not when it is queued.
It may capture `Rc` inputs if the future it returns is itself `Send`.
This probe uses `Rc<Cell<u32>>` for the input and records admitted snapshots:

<!-- shown: advanced tasks: factory probe -->
```rust
#[derive(Debug)]
struct Probe {
    input: Rc<Cell<u32>>,
    started: Rc<RefCell<Vec<String>>>,
    push: Push<Deltas>,
}

struct Schedule;
struct Produced(u32);

actor! {
    Probe {
        handlers {
            Schedule => { task Produced }
            Produced
        }
    }
}
```
<!-- /shown -->

<!-- shown: advanced tasks: a deferred factory takes its snapshot on admission -->
```rust
#[handler]
fn schedule(In(this, _, cx): In<Probe, Schedule>) {
    let input = this.input.clone();
    let started = this.started.clone();
    cx.task().conflate().spawn_with(move |cancel| {
        let snapshot = input.get();
        started.borrow_mut().push(format!("start {snapshot}"));
        async move {
            let _ = cancel
                .guard(tokio::time::sleep(Duration::from_secs(1)))
                .await;
            Produced(snapshot)
        }
    });
}
```
<!-- /shown -->

`input` is an `Rc<Cell<u32>>`; `started` records factory calls and reply
handling. A queued
factory reads the value at admission, which can differ from its request-time
value. For a request-time snapshot, read before calling `spawn_with` and
move that owned snapshot into the closure instead.

<!-- shown: advanced tasks: factory installation -->
```rust
#[handler]
fn produced(In(this, Produced(value), _): In<Probe, Produced>) {
    this.started.borrow_mut().push(format!("reply {value}"));
    this.push.send(value);
}

feature! {
    pub Probing {
        exports { Deltas }
    }
}

#[installs]
fn probing(cx: &Install<(Rc<Cell<u32>>, Rc<RefCell<Vec<String>>>)>) -> anyhow::Result<Probing> {
    let (deltas, _) = cx.state::<Deltas>().driven_by(|push| Probe {
        input: cx.params().0.clone(),
        started: cx.params().1.clone(),
        push,
    });
    deltas.emit(Schedule);
    Ok(Probing(deltas))
}
```
<!-- /shown -->

<!-- shown: advanced tasks: deferred input is not a request time snapshot -->
```rust
#[guinea::test]
fn a_waiting_factory_observes_input_only_when_it_is_started(h: &mut Harness) {
    let input = Rc::new(Cell::new(1));
    let started = Rc::new(RefCell::new(Vec::new()));
    h.install::<Probing>(&(input.clone(), started.clone()))
        .unwrap();
    h.settled();
    input.set(2);
    h.dispatch::<Deltas>().emit(Schedule);
    input.set(9);
    assert_eq!(*started.borrow(), ["start 1"]);

    h.advance(Duration::from_secs(1));
    assert_eq!(*started.borrow(), ["start 1", "start 9", "reply 1"]);
    assert_eq!(h.state::<Deltas>().0, [1]);
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Deltas>().0, [1, 9]);
    assert_eq!(
        *started.borrow(),
        ["start 1", "start 9", "reply 1", "reply 9"]
    );
}
```
<!-- /shown -->

Here `Probing` claims `Deltas`, installs that actor with the two supplied
cells and schedules the first run. Its `Produced` handler appends the
result. The pending request is made at `2`, but starts with `9`.
The asserted log also puts `start 9` before `reply 1`: admitting the next
factory does not wait for the previous answer's handler.

Sleep does not suspend this lane or its replies:

<!-- shown: advanced tasks: conflation while asleep -->
```rust
#[guinea::test]
fn conflate_starts_pending_work_and_handles_answers_while_asleep(h: &mut Harness) {
    let input = Rc::new(Cell::new(1));
    let started = Rc::new(RefCell::new(Vec::new()));
    let page = h.child();
    page.install::<Probing>(&(input.clone(), started.clone()))
        .unwrap();
    h.settled();
    input.set(2);
    page.dispatch::<Deltas>().emit(Schedule);
    page.scope().sleep();
    h.advance(Duration::from_secs(1));
    assert_eq!(*started.borrow(), ["start 1", "start 2", "reply 1"]);
    h.advance(Duration::from_secs(1));
    assert_eq!(page.state::<Deltas>().0, [1, 2]);
}
```
<!-- /shown -->

## A calculator that must return before the next run

Rate calculations often carry working state: the previous counters and
timestamps. Move that state into a task, then return it with the result.
Only the newest input received meanwhile needs another calculation.

Do **not** assume that a `conflate` factory runs after the previous reply
handler restores the calculator. The lane admits its next run before that
reply is handled. Taking an `Option<Calculator>` out of actor state at that
point would not be a valid ownership handoff.

An explicit actor state machine gives that handoff a clear order:

<!-- shown: advanced tasks: calculation state travels with the work -->
```rust
#[derive(Default, Debug)]
struct Calculator {
    previous: u32,
}

pub struct Sample(pub u32);
pub struct Calculated {
    calculator: Calculator,
    delta: u32,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Deltas(pub Vec<u32>);

#[reducer]
fn deltas(this: &mut Deltas, delta: u32) {
    this.0.push(delta);
}

#[derive(Debug)]
struct Computing {
    calculator: Option<Calculator>,
    pending: Option<u32>,
    push: Push<Deltas>,
}

actor! {
    Computing {
        handlers {
            Sample => { task Calculated }
            Calculated => { send Sample loop }
        }
    }
}

async fn calculate(mut calculator: Calculator, snapshot: u32) -> Calculated {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let delta = snapshot.saturating_sub(calculator.previous);
    calculator.previous = snapshot;
    Calculated { calculator, delta }
}

#[handler]
fn sample(In(this, Sample(snapshot), cx): In<Computing, Sample>) {
    if let Some(calculator) = this.calculator.take() {
        cx.task().spawn(calculate(calculator, snapshot));
    } else {
        this.pending = Some(snapshot);
    }
}

#[handler]
fn calculated(In(this, result, cx): In<Computing, Calculated>) {
    this.calculator = Some(result.calculator);
    this.push.send(result.delta);
    if let Some(snapshot) = this.pending.take() {
        cx.send(Sample(snapshot));
    }
}

feature! {
    pub Calculating {
        exports { Deltas }
    }
}

#[installs]
fn calculating(cx: &Install) -> anyhow::Result<Calculating> {
    let (deltas, _) = cx.state::<Deltas>().driven_by(|push| Computing {
        calculator: Some(Calculator::default()),
        pending: None,
        push,
    });
    Ok(Calculating(deltas))
}
```
<!-- /shown -->

This implements conflation without calling `.conflate()`: one active task,
one replaceable pending snapshot. The result handler first restores the
calculator, then queues `Sample` to its own actor. The `loop` annotation
marks this intentional message cycle; it does not create the loop by itself.
`calculate` owns all its inputs and contains no reference to the actor.

<!-- shown: advanced tasks: conflating while preserving calculation state -->
```rust
#[guinea::test]
fn only_the_last_pending_sample_runs_with_the_returned_calculator(h: &mut Harness) {
    h.install::<Calculating>(&()).unwrap();
    let dispatch = h.dispatch::<Deltas>();
    dispatch.emit(Sample(10));
    h.settled();
    dispatch.emit(Sample(20));
    dispatch.emit(Sample(30));

    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Deltas>().0, [10]);
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Deltas>().0, [10, 20]);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

The input `20` never starts. The second delta is `30 - 10`, proving that
the calculator returned by the first task was reused. The pending snapshot
is the latest input captured by `Sample`, not a later re-read of shared
state. Handle failure and cancellation explicitly if the actor must recover
its calculator without being removed; dropping an in-flight future does
not magically return moved state to the actor.

## Helpers and `Cx<A, M>`

`M` identifies the handler's incoming message. It is not optional context
metadata: with a declared graph, it controls which outgoing messages that
handler may send or produce. A helper fixed to `Cx<A>` means `Cx<A, ()>`,
not “any handler of A.”

A helper which only captures an async context can be generic:

<!-- shown: advanced tasks: generic context helper -->
```rust
fn capture_context<A: 'static, M>(cx: &Cx<A, M>) -> AsyncContext<A> {
    cx.async_ctx()
}
```
<!-- /shown -->

For calculation helpers, the simpler public boundary is often the owned
future, as `calculate` above demonstrates. Keep `cx.task().spawn(...)` or
`cx.send(...)` at the concrete handler where its message type is known.

There is a current public-API limitation: the `Allows<M, Out>` bound used by
generic sending/spawning is not exported through the supported public actor
API; it is exposed only for macro expansion under `__private`. This book
does not recommend depending on that hidden path. It cannot offer a stable
fully generic helper requiring that bound until it has a public export.
