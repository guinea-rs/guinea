use guinea::app::Harness;
use guinea::prelude::*;

//@show event bus: a shared contract
#[derive(Clone, Debug, Event)]
pub struct RunInProcess;

pub struct Start;
//@show-end

//@show event bus: publishing from an actor
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
//@show-end

//@show event bus: subscribing an actor
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
//@show-end

//@show event bus: actors in sibling scopes communicate
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
//@show-end

//@show event bus: every subscriber hears it
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
//@show-end

//@show event bus: sleeping subscribers miss events
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
//@show-end

#[guinea::test]
fn publishing_without_a_subscriber_does_not_acknowledge_execution(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    h.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(h.state::<Requests>().0, 1);
}

//@show event bus: a scoped callback
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
//@show-end

#[guinea::test]
fn a_callback_hears_the_same_event_as_an_actor(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    h.install::<Observing>(&()).unwrap();
    let backend = h.child();
    backend.install::<AgentRuntime>(&()).unwrap();

    h.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(h.state::<Notices>().0, 1);
    assert_eq!(backend.state::<Runs>().0, 1);
}

#[guinea::test]
fn leaving_removes_the_actor_subscription(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    h.install::<Observing>(&()).unwrap();
    let backend = h.child();
    backend.install::<AgentRuntime>(&()).unwrap();
    assert_eq!(h.bus().listeners().len(), 1);

    backend.leave();
    h.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(h.state::<Notices>().0, 1);
    let listeners = h.bus().listeners();
    assert_eq!(listeners[0].listeners.len(), 1);
}
