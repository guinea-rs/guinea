use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use guinea::app::Harness;
use guinea::prelude::*;

//@show advanced tasks: calculation state travels with the work
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
//@show-end

//@show advanced tasks: conflating while preserving calculation state
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
//@show-end

//@show advanced tasks: generic context helper
fn capture_context<A: 'static, M>(cx: &Cx<A, M>) -> AsyncContext<A> {
    cx.async_ctx()
}
//@show-end

//@show advanced tasks: factory probe
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
//@show-end

//@show advanced tasks: a deferred factory takes its snapshot on admission
#[handler]
fn schedule(In(this, _, cx): In<Probe, Schedule>) {
    let input = this.input.clone();
    let started = this.started.clone();
    //@hide
    assert!(!capture_context(&cx).is_cancelled());
    //@unhide
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
//@show-end

//@show advanced tasks: factory installation
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
//@show-end

//@show advanced tasks: deferred input is not a request time snapshot
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
//@show-end

//@show advanced tasks: conflation while asleep
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
//@show-end
