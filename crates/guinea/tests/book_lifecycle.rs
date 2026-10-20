use std::time::Duration;

use guinea::app::Harness;
use guinea::prelude::*;

//@show lifecycle: ordinary work and wake hooks
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
//@show-end

//@show lifecycle: a task can answer during sleep
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
//@show-end

//@show lifecycle: wake hooks follow registration order
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
//@show-end

#[guinea::test]
fn an_explicit_send_and_its_queued_followup_are_not_gated_by_sleep(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    page.scope().sleep();
    page.dispatch::<Log>().emit(Queue);
    h.settled();
    assert_eq!(page.state::<Log>().0, ["answer"]);
}

#[guinea::test]
fn removing_the_scope_cancels_its_pending_ordinary_work(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    page.dispatch::<Log>().emit(Begin);
    page.dispatch::<Log>().emit(Direct);
    h.settled();
    assert_eq!(h.stuck(), 2);
    page.leave();
    h.advance(Duration::from_secs(1));
    assert_eq!(h.stuck(), 0);
}

//@show lifecycle: latest while asleep
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
//@show-end
