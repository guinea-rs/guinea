//! The examples in `#[derive(Request)]`'s documentation, run. `cargo xtask
//! docs` copies what is between the marks into it.

use std::time::Duration;

use guinea::app::Harness;
use guinea::core::actor::event_bus::RpcRequest;
use guinea::prelude::*;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Said(pub Option<String>);

#[reducer]
fn said(this: &mut Said, said: String) {
    this.0 = Some(said);
}

pub mod contracts {
    //@show a request and its reply
    #[derive(Clone, Debug, guinea::Request)]
    #[request(reply = Outcome)]
    pub struct Kill(pub u32);

    #[derive(Clone, Debug, PartialEq)]
    pub enum Outcome {
        Done,
        Denied,
    }
    //@show-end
}

use contracts::{Kill, Outcome};

mod answering {
    use super::*;

    //@show the one that answers
    #[derive(Debug, Default)]
    pub struct Processes {
        protected: Vec<u32>,
    }

    actor! {
        Processes {
            handlers { RpcRequest<Kill> }
            subscribes { RpcRequest<Kill> }
        }
    }

    // Returning the reply is what makes it the answer: the generated code
    // sends it back, once.
    #[handler]
    fn kill(this: &mut Processes, Kill(pid): Kill) -> Outcome {
        match this.protected.contains(&pid) {
            true => Outcome::Denied,
            false => Outcome::Done,
        }
    }

    feature! {
        pub Killing {}
    }

    // It answers for as long as the segment that installed it stands.
    #[installs]
    fn killing(cx: &FeatureInitContext) -> anyhow::Result<Killing> {
        cx.spawn(Processes { protected: vec![4] });
        Ok(Killing)
    }
    //@show-end
}

mod answering_later {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Processes {
        protected: Vec<u32>,
    }

    actor! {
        Processes {
            handlers { RpcRequest<Kill> }
            subscribes { RpcRequest<Kill> }
        }
    }

    #[handler]
    fn kill(this: &mut Processes, Kill(pid): Kill) -> Reply<Outcome> {
        let protected = this.protected.contains(&pid);

        Reply::later(async move {
            guinea::core::executor::yield_now().await;

            match protected {
                true => Outcome::Denied,
                false => Outcome::Done,
            }
        })
    }

    feature! {
        pub Killing {}
    }

    #[installs]
    fn killing(cx: &FeatureInitContext) -> anyhow::Result<Killing> {
        cx.spawn(Processes { protected: vec![4] });
        Ok(Killing)
    }
}

mod answering_and_sending {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct LastKilled(pub Option<u32>);

    #[reducer]
    fn last_killed(this: &mut LastKilled, pid: u32) {
        this.0 = Some(pid);
    }

    pub struct Killed(pub u32);

    #[derive(Debug)]
    pub struct Processes {
        push: Push<LastKilled>,
    }

    actor! {
        Processes {
            handlers {
                RpcRequest<Kill> => { send Killed },
                Killed,
            }
            subscribes { RpcRequest<Kill> }
        }
    }

    #[handler]
    fn kill(_this: &mut Processes, Kill(pid): Kill, cx: Cx) -> Outcome {
        cx.send(Killed(pid));
        Outcome::Done
    }

    #[handler]
    fn killed(this: &mut Processes, Killed(pid): Killed) {
        this.push.send(pid);
    }

    feature! {
        pub Killing {
            exports { LastKilled }
        }
    }

    #[installs]
    fn killing(cx: &FeatureInitContext) -> anyhow::Result<Killing> {
        let (killed, _) = cx.state::<LastKilled>().driven_by(|push| Processes { push });
        Ok(Killing(killed))
    }
}

mod asking {
    use super::*;

    //@show asking
    pub struct Ask(pub u32);
    pub struct Told(pub String);

    #[derive(Debug)]
    pub struct Asker {
        push: Push<Said>,
    }

    actor! {
        Asker {
            handlers { Ask, Told }
        }
    }

    // Asked off the UI thread, and answered with what the one answerer
    // returned - or with why there was nothing to wait for.
    #[handler]
    async fn ask(cx: AsyncContext<Asker>, Ask(pid): Ask) {
        let said = match cx.request(Kill(pid), Duration::from_secs(5)).await {
            Ok(outcome) => format!("{outcome:?}"),
            Err(error) => error.to_string(),
        };
        cx.send(Told(said));
    }

    #[handler]
    fn told(this: &mut Asker, Told(said): Told) {
        this.push.send(said);
    }
    //@show-end

    feature! {
        pub Asking {
            exports { Said }
        }
    }

    #[installs]
    fn asking(cx: &FeatureInitContext) -> anyhow::Result<Asking> {
        let (said, _) = cx.state::<Said>().driven_by(|push| Asker { push });
        Ok(Asking(said))
    }
}

mod listening {
    use super::*;

    //@show one that only hears it
    #[derive(Debug, Default)]
    pub struct Audit {
        pub seen: Vec<u32>,
    }

    actor! {
        Audit {
            handlers { RpcRequest<Kill> }
            subscribes { RpcRequest<Kill> }
        }
    }

    // Returns nothing, so it hears the request and cannot answer it: there
    // is one answerer, and it is not this.
    #[handler]
    fn heard(this: &mut Audit, request: RpcRequest<Kill>) {
        this.seen.push(request.payload.0);
    }

    feature! {
        pub Auditing {}
    }

    #[installs]
    fn auditing(cx: &FeatureInitContext) -> anyhow::Result<Auditing> {
        cx.spawn(Audit::default());
        Ok(Auditing)
    }
    //@show-end
}

mod answering_async {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Processes;

    actor! {
        Processes {
            handlers { RpcRequest<Kill> }
            subscribes { RpcRequest<Kill> }
        }
    }

    #[handler]
    async fn kill(_cx: AsyncContext<Processes>, Kill(pid): Kill) -> Outcome {
        guinea::core::executor::yield_now().await;

        match pid {
            4 => Outcome::Denied,
            _ => Outcome::Done,
        }
    }

    feature! {
        pub Killing {}
    }

    #[installs]
    fn killing(cx: &FeatureInitContext) -> anyhow::Result<Killing> {
        cx.spawn(Processes);
        Ok(Killing)
    }
}

mod dropping {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Processes;

    actor! {
        Processes {
            handlers { RpcRequest<Kill> }
            subscribes { RpcRequest<Kill> }
        }
    }

    impl Handler<RpcRequest<Kill>> for Processes {
        const ANSWERS: bool = true;

        fn handle(&mut self, _: RpcRequest<Kill>, _cx: Cx<Self, RpcRequest<Kill>>) {}
    }

    feature! {
        pub Killing {}
    }

    #[installs]
    fn killing(cx: &FeatureInitContext) -> anyhow::Result<Killing> {
        cx.spawn(Processes);
        Ok(Killing)
    }
}

mod listening_and_replying {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Heard(pub Vec<u32>);

    #[reducer]
    fn heard(this: &mut Heard, pid: u32) {
        this.0.push(pid);
    }

    #[derive(Debug)]
    pub struct Audit {
        push: Push<Heard>,
    }

    actor! {
        Audit {
            handlers { RpcRequest<Kill> }
            subscribes { RpcRequest<Kill> }
        }
    }

    #[handler]
    fn hearing(this: &mut Audit, request: RpcRequest<Kill>) {
        this.push.send(request.payload.0);
        request.reply(Outcome::Denied);
    }

    feature! {
        pub Auditing {
            exports { Heard }
        }
    }

    #[installs]
    fn auditing(cx: &FeatureInitContext) -> anyhow::Result<Auditing> {
        let (heard, _) = cx.state::<Heard>().driven_by(|push| Audit { push });
        Ok(Auditing(heard))
    }
}

mod cycle {
    use super::*;

    #[derive(Clone, Debug, guinea::Request)]
    #[request(reply = Looped)]
    pub struct Loop;

    #[derive(Clone, Debug)]
    pub struct Looped;

    #[derive(Clone, Debug, guinea::Request)]
    #[request(reply = Backed)]
    pub struct Back;

    #[derive(Clone, Debug)]
    pub struct Backed;

    #[derive(Debug, Default)]
    pub struct There;

    #[derive(Debug, Default)]
    pub struct Again;

    actor! {
        There {
            handlers { RpcRequest<Loop> }
            subscribes { RpcRequest<Loop> }
        }
    }

    actor! {
        Again {
            handlers { RpcRequest<Back> }
            subscribes { RpcRequest<Back> }
        }
    }

    #[handler]
    async fn there(cx: AsyncContext<There>, _: Loop) -> Looped {
        let _ = cx.request(Back, Duration::from_secs(5)).await;
        Looped
    }

    #[handler]
    async fn again(cx: AsyncContext<Again>, _: Back) -> Backed {
        let _ = cx.request(Loop, Duration::from_secs(5)).await;
        Backed
    }

    feature! {
        pub Looping {}
    }

    #[installs]
    fn looping(cx: &FeatureInitContext) -> anyhow::Result<Looping> {
        cx.spawn(There);
        cx.spawn(Again);
        Ok(Looping)
    }
}

fn asked(h: &Harness, pid: u32) -> Option<String> {
    h.dispatch::<Said>().emit(asking::Ask(pid));
    h.settled();
    h.state::<Said>().0.clone()
}

#[guinea::test(iterations = 4)]
fn the_answer_comes_back_to_whoever_asked(h: &mut Harness) {
    h.install::<answering::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    assert_eq!(asked(h, 7).as_deref(), Some("Done"));
    assert_eq!(asked(h, 4).as_deref(), Some("Denied"));
}

#[guinea::test(iterations = 4)]
fn an_answer_given_later_comes_back_to_whoever_asked(h: &mut Harness) {
    h.install::<answering_later::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    assert_eq!(asked(h, 7).as_deref(), Some("Done"));
    assert_eq!(asked(h, 4).as_deref(), Some("Denied"));
}

#[guinea::test(iterations = 4)]
fn an_answerer_sends_what_its_request_declares(h: &mut Harness) {
    h.install::<answering_and_sending::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    assert_eq!(asked(h, 7).as_deref(), Some("Done"));
    assert_eq!(h.state::<answering_and_sending::LastKilled>().0, Some(7));
}

#[guinea::test(iterations = 4)]
fn a_request_nobody_answers_says_so_at_once(h: &mut Harness) {
    h.install::<listening::Auditing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    let said = asked(h, 7).expect("an answer, not a wait for the timeout");
    assert!(said.starts_with("nobody answers"), "{said}");
}

#[guinea::test(iterations = 4)]
fn a_listener_beside_the_answerer_does_not_change_the_answer(h: &mut Harness) {
    h.install::<listening::Auditing>(&()).unwrap();
    h.install::<answering::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    assert_eq!(asked(h, 4).as_deref(), Some("Denied"));
}

#[guinea::test(iterations = 4)]
fn the_answerer_goes_with_its_segment(h: &mut Harness) {
    h.install::<asking::Asking>(&()).unwrap();

    let area = h.child();
    area.install::<answering::Killing>(&()).unwrap();
    assert_eq!(asked(h, 7).as_deref(), Some("Done"));

    area.leave();
    let said = asked(h, 7).expect("an answer, not a wait for the timeout");
    assert!(said.starts_with("nobody answers"), "{said}");
}

#[guinea::test(iterations = 4)]
fn an_answerer_written_async_answers_with_what_it_returns(h: &mut Harness) {
    h.install::<answering_async::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    assert_eq!(asked(h, 7).as_deref(), Some("Done"));
    assert_eq!(asked(h, 4).as_deref(), Some("Denied"));
}

#[guinea::test(iterations = 4)]
fn a_listener_hears_the_request_and_its_reply_goes_nowhere(h: &mut Harness) {
    h.install::<listening_and_replying::Auditing>(&()).unwrap();
    h.install::<answering::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    assert_eq!(asked(h, 7).as_deref(), Some("Done"));
    assert_eq!(h.state::<listening_and_replying::Heard>().0, [7]);
}

#[guinea::test(iterations = 4)]
fn the_answerer_s_place_is_free_again_once_its_segment_goes(h: &mut Harness) {
    h.install::<asking::Asking>(&()).unwrap();

    let first = h.child();
    first.install::<answering::Killing>(&()).unwrap();
    first.leave();

    let second = h.child();
    second.install::<answering::Killing>(&()).unwrap();
    assert_eq!(asked(h, 7).as_deref(), Some("Done"));
}

#[guinea::test(iterations = 4)]
fn a_request_whose_answerer_sleeps_says_so_at_once(h: &mut Harness) {
    h.install::<asking::Asking>(&()).unwrap();

    let area = h.child();
    area.install::<answering::Killing>(&()).unwrap();
    area.context().scope.sleep();

    let said = asked(h, 7).expect("an answer, not a wait for the timeout");
    assert!(said.contains("is asleep"), "{said}");
}

#[guinea::test(iterations = 4)]
fn a_request_its_answerer_drops_says_so_at_once(h: &mut Harness) {
    h.install::<dropping::Killing>(&()).unwrap();
    h.install::<asking::Asking>(&()).unwrap();

    let said = asked(h, 7).expect("an answer, not a wait for the timeout");
    assert!(said.contains("without an answer"), "{said}");
}

#[guinea::test(iterations = 4)]
fn a_request_that_waits_on_itself_panics_where_it_closes_the_loop(h: &mut Harness) {
    h.install::<cycle::Looping>(&()).unwrap();
    let scope = h.segment().context().scope;

    guinea::core::executor::spawn(async move {
        let _ = scope.request(cycle::Loop, Duration::from_secs(5)).await;
    });
    let settled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.settled()));

    let panic = settled.expect_err("the loop closed without a word");
    let said = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .unwrap_or_default();
    assert!(said.contains("RPC cycle detected"), "{said}");
}

#[test]
#[should_panic(expected = "a request has exactly one answerer")]
fn a_second_answerer_is_refused() {
    let h = Harness::new(0);
    h.install::<answering::Killing>(&()).unwrap();
    h.child().install::<answering::Killing>(&()).unwrap();
}
