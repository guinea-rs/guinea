use std::marker::PhantomData;

use guinea::app::Harness;
use guinea::prelude::*;

pub trait Backend: 'static {
    type Value: Send + 'static;

    fn value() -> Self::Value;
    fn number(value: Self::Value) -> usize;
}

pub struct Small;

impl Backend for Small {
    type Value = u8;

    fn value() -> u8 {
        5
    }

    fn number(value: u8) -> usize {
        usize::from(value)
    }
}

pub struct Large;

impl Backend for Large {
    type Value = u16;

    fn value() -> u16 {
        300
    }

    fn number(value: u16) -> usize {
        usize::from(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals(pub Vec<usize>);

#[reducer]
fn totals(this: &mut Totals, value: usize) {
    this.0.push(value);
}

feature! {
    pub Recording {
        exports { Totals }
    }
}

#[installs]
fn recording(cx: &Install) -> anyhow::Result<Recording> {
    Ok(Recording(cx.state::<Totals>().plain()))
}

pub struct Start;
pub struct Connected<Value>(Value);
pub struct Wrapped<BackendType: Backend>(BackendType::Value);
pub struct Finished(usize);

#[derive(Clone, Event)]
pub struct Refresh;

struct Fetcher<BackendType: Backend, const OFFSET: usize> {
    push: Push<Totals>,
    backend: PhantomData<BackendType>,
}

actor! {
    Fetcher<BackendType, const OFFSET: usize> where BackendType: Backend {
        handlers {
            Start => { task Connected<BackendType::Value> },
            Connected<BackendType::Value> => { send Finished },
            Wrapped<BackendType> => { send Finished },
            Finished
        }
        subscribes { Refresh => { task Wrapped<BackendType> } }
    }
}

#[handler]
fn start<BackendType: Backend, const OFFSET: usize>(
    In(_, _, cx): In<Fetcher<BackendType, OFFSET>, Start>,
) {
    cx.task().spawn(async { Connected(BackendType::value()) });
}

#[handler]
fn connected<BackendType: Backend, const OFFSET: usize>(
    In(_, Connected(value), cx): In<Fetcher<BackendType, OFFSET>, Connected<BackendType::Value>>,
) {
    cx.send(Finished(BackendType::number(value) + OFFSET));
}

#[handler]
fn refresh<BackendType: Backend, const OFFSET: usize>(
    In(_, _, cx): In<Fetcher<BackendType, OFFSET>, Refresh>,
) {
    cx.task()
        .spawn(async { Wrapped::<BackendType>(BackendType::value()) });
}

#[handler]
fn wrapped<BackendType: Backend, const OFFSET: usize>(
    In(_, Wrapped(value), cx): In<Fetcher<BackendType, OFFSET>, Wrapped<BackendType>>,
) {
    cx.send(Finished(BackendType::number(value) + OFFSET));
}

#[handler]
fn finished<BackendType: Backend, const OFFSET: usize>(
    In(this, Finished(value), _): In<Fetcher<BackendType, OFFSET>, Finished>,
) {
    this.push.send(value);
}

fn fetching<BackendType: Backend, const OFFSET: usize>(
    harness: &mut Harness,
) -> Addr<Fetcher<BackendType, OFFSET>> {
    harness.install::<Recording>(&()).unwrap();
    let push = Push::new(harness.segment().scope());
    harness
        .app(|app| {
            Ok(app.spawn(Fetcher::<BackendType, OFFSET> {
                push,
                backend: PhantomData,
            }))
        })
        .unwrap()
}

#[guinea::test]
fn generic_flow_keeps_projected_task_outputs_and_owned_messages(h: &mut Harness) {
    let actor = fetching::<Small, 3>(h);
    actor.send(Start);
    h.settled();
    assert_eq!(h.state::<Totals>().0, [8]);

    h.publish(Refresh);
    h.settled();
    assert_eq!(h.state::<Totals>().0, [8, 8]);
}

#[guinea::test]
fn another_backend_and_const_argument_get_their_own_flow(h: &mut Harness) {
    let actor = fetching::<Large, 7>(h);
    actor.send(Start);
    h.settled();
    assert_eq!(h.state::<Totals>().0, [307]);

    h.publish(Refresh);
    h.settled();
    assert_eq!(h.state::<Totals>().0, [307, 307]);
}
