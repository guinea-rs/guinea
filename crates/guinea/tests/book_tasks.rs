use std::time::Duration;

use guinea::app::Harness;
use guinea::prelude::*;

//@show tasks: state and messages
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Results {
    pub query: String,
    pub names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub query: String,
    pub names: Vec<String>,
}

#[reducer]
fn results(this: &mut Results, found: Found) {
    this.query = found.query;
    this.names = found.names;
}

pub struct Query(pub String);
pub struct CancelSearch;
//@show-end

//@show tasks: simulated search
async fn search(query: String) -> Found {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let names = ["guinea", "guinea-app", "shown"]
        .into_iter()
        .filter(|name| name.contains(query.as_str()))
        .map(str::to_owned)
        .collect();
    Found { query, names }
}
//@show-end

//@show tasks: an actor starts work
#[derive(Debug)]
struct Searching {
    push: Push<Results>,
}

actor! {
    Searching {
        handlers { Query, CancelSearch }
    }
}

#[handler]
fn query(In(this, Query(text), cx): In<Searching, Query>) {
    cx.task().latest().pushing(this.push).spawn(search(text));
}
//@show-end

//@show tasks: cancelling a lane
#[handler]
fn cancel_search(In(_, _, cx): In<Searching, CancelSearch>) {
    cx.task().lane::<Found>().cancel();
}
//@show-end

//@show tasks: installation
feature! {
    pub Search {
        exports { Results }
    }
}

#[installs]
fn searching(cx: &Install) -> anyhow::Result<Search> {
    let (results, _) = cx.state::<Results>().driven_by(|push| Searching { push });
    Ok(Search(results))
}
//@show-end

//@show tasks: only the newest query answers
#[guinea::test]
fn a_new_query_replaces_the_search_in_flight(h: &mut Harness) {
    h.install::<Search>(&()).unwrap();
    let dispatch = h.dispatch::<Results>();

    dispatch.emit(Query("shown".into()));
    h.settled();
    h.advance(Duration::from_millis(500));

    dispatch.emit(Query("guinea".into()));
    h.settled();
    h.advance(Duration::from_millis(500));
    assert_eq!(*h.state::<Results>(), Results::default());

    h.advance(Duration::from_millis(500));
    let results = h.state::<Results>();
    assert_eq!(results.query, "guinea");
    assert_eq!(results.names, ["guinea", "guinea-app"]);
}
//@show-end

//@show tasks: an explicit cancellation
#[guinea::test]
fn a_cancelled_search_does_not_change_the_results(h: &mut Harness) {
    h.install::<Search>(&()).unwrap();
    let dispatch = h.dispatch::<Results>();
    dispatch.emit(Query("guinea".into()));
    h.settled();

    dispatch.emit(CancelSearch);
    h.advance(Duration::from_secs(2));

    assert_eq!(*h.state::<Results>(), Results::default());
    assert_eq!(h.stuck(), 0);

    dispatch.emit(Query("shown".into()));
    h.settled();
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Results>().query, "shown");
}
//@show-end

//@show tasks: leaving cancels the work
#[guinea::test]
fn leaving_the_page_cancels_its_search(h: &mut Harness) {
    let page = h.child();
    page.install::<Search>(&()).unwrap();
    page.dispatch::<Results>().emit(Query("guinea".into()));
    h.settled();
    assert_eq!(h.stuck(), 1);

    page.leave();
    h.settled();
    assert_eq!(h.stuck(), 0);
}
//@show-end

mod replied {
    use super::*;

    //@show tasks: replying through the actor
    #[derive(Debug)]
    struct Searching {
        push: Push<Results>,
    }

    actor! {
        Searching {
            handlers {
                Query => { task Found }
                Found,
                CancelSearch
            }
        }
    }

    #[handler]
    fn query(In(_, Query(text), cx): In<Searching, Query>) {
        cx.task().latest().spawn(search(text));
    }

    #[handler]
    fn found(In(this, answer, _): In<Searching, Found>) {
        this.push.send(answer);
    }
    //@show-end

    #[handler]
    fn cancel_search(In(_, _, cx): In<Searching, CancelSearch>) {
        cx.task().lane::<Found>().cancel();
    }

    feature! {
        pub Search {
            exports { Results }
        }
    }

    #[installs]
    fn searching(cx: &Install) -> anyhow::Result<Search> {
        let (results, _) = cx.state::<Results>().driven_by(|push| Searching { push });
        Ok(Search(results))
    }
}

#[guinea::test]
fn pushing_and_replying_have_the_same_search_behavior(h: &mut Harness) {
    let direct = h.child();
    let replied = h.child();
    direct.install::<Search>(&()).unwrap();
    replied.install::<replied::Search>(&()).unwrap();
    let dispatchers = [direct.dispatch::<Results>(), replied.dispatch::<Results>()];

    for dispatch in dispatchers {
        dispatch.emit(Query("shown".into()));
    }
    h.settled();
    h.advance(Duration::from_millis(500));
    for dispatch in dispatchers {
        dispatch.emit(Query("guinea".into()));
    }
    h.settled();
    h.advance(Duration::from_millis(500));
    assert_eq!(*direct.state::<Results>(), Results::default());
    assert_eq!(direct.state::<Results>(), replied.state::<Results>());

    h.advance(Duration::from_millis(500));
    assert_eq!(direct.state::<Results>().query, "guinea");
    assert_eq!(direct.state::<Results>().names, ["guinea", "guinea-app"]);
    assert_eq!(direct.state::<Results>(), replied.state::<Results>());

    for dispatch in dispatchers {
        dispatch.emit(Query("shown".into()));
    }
    h.settled();
    for dispatch in dispatchers {
        dispatch.emit(CancelSearch);
    }
    h.advance(Duration::from_secs(1));
    assert_eq!(direct.state::<Results>().query, "guinea");
    assert_eq!(direct.state::<Results>(), replied.state::<Results>());
    assert_eq!(h.stuck(), 0);
}

mod cooperative {
    use super::*;

    //@show tasks: a session closes asynchronously
    use tokio::sync::oneshot;

    #[derive(Debug)]
    pub struct Session {
        close_requested: oneshot::Sender<()>,
        closed: oneshot::Receiver<()>,
    }

    impl Session {
        async fn close(self) -> bool {
            if self.close_requested.send(()).is_err() {
                return false;
            }
            self.closed.await.is_ok()
        }
    }

    pub fn session_pair() -> (Session, oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (close_requested, closing) = oneshot::channel();
        let (confirm_closed, closed) = oneshot::channel();
        (
            Session {
                close_requested,
                closed,
            },
            closing,
            confirm_closed,
        )
    }
    //@show-end

    //@show tasks: cooperative work
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Completed(pub u32);

    #[derive(Clone)]
    pub struct SessionFinished(pub bool);

    #[reducer]
    fn completed(this: &mut Completed, finished: SessionFinished) {
        if finished.0 {
            this.0 += 1;
        }
    }

    pub struct StartSession(pub Session);
    pub struct StopSession;

    #[derive(Debug)]
    struct Working {
        push: Push<Completed>,
    }

    actor! {
        Working {
            handlers { StartSession, StopSession }
        }
    }

    #[handler]
    fn start_session(In(this, StartSession(session), cx): In<Working, StartSession>) {
        cx.task()
            .latest()
            .pushing(this.push)
            .spawn_with(|cancel| async move {
                let _ = cancel
                    .guard(tokio::time::sleep(Duration::from_secs(1)))
                    .await;
                SessionFinished(session.close().await)
            });
    }

    #[handler]
    fn stop_session(In(_, _, cx): In<Working, StopSession>) {
        cx.task().lane::<SessionFinished>().cancel();
    }

    feature! {
        pub SessionWork {
            exports { Completed }
        }
    }

    #[installs]
    fn session_work(cx: &Install) -> anyhow::Result<SessionWork> {
        let (completed, _) = cx.state::<Completed>().driven_by(|push| Working { push });
        Ok(SessionWork(completed))
    }
    //@show-end
}

use cooperative::{Completed, SessionWork, StartSession, StopSession, session_pair};

//@show tasks: cancellation lets cleanup finish but drops the answer
#[guinea::test]
fn cancellation_waits_for_session_cleanup_without_applying_its_answer(h: &mut Harness) {
    let (session, mut closing, confirm_closed) = session_pair();
    h.install::<SessionWork>(&()).unwrap();
    let dispatch = h.dispatch::<Completed>();
    dispatch.emit(StartSession(session));
    h.settled();
    assert_eq!(
        closing.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );

    dispatch.emit(StopSession);
    h.settled();
    assert_eq!(closing.try_recv(), Ok(()));
    assert_eq!(h.stuck(), 1);
    assert_eq!(h.state::<Completed>().0, 0);

    confirm_closed.send(()).unwrap();
    h.settled();
    assert_eq!(h.stuck(), 0);
    assert_eq!(h.state::<Completed>().0, 0);
}
//@show-end

//@show tasks: leaving also allows cooperative cleanup
#[guinea::test]
fn session_cleanup_can_finish_after_its_page_leaves(h: &mut Harness) {
    let (session, mut closing, confirm_closed) = session_pair();
    let page = h.child();
    page.install::<SessionWork>(&()).unwrap();
    page.dispatch::<Completed>().emit(StartSession(session));
    h.settled();

    page.leave();
    h.settled();
    assert_eq!(closing.try_recv(), Ok(()));
    assert_eq!(h.stuck(), 1);

    confirm_closed.send(()).unwrap();
    h.settled();
    assert_eq!(h.stuck(), 0);
}
//@show-end

#[guinea::test]
fn an_uncancelled_session_applies_its_answer_after_cleanup(h: &mut Harness) {
    let (session, mut closing, confirm_closed) = session_pair();
    h.install::<SessionWork>(&()).unwrap();
    h.dispatch::<Completed>().emit(StartSession(session));
    h.settled();
    h.advance(Duration::from_secs(1));
    assert_eq!(closing.try_recv(), Ok(()));
    assert_eq!(h.state::<Completed>().0, 0);

    confirm_closed.send(()).unwrap();
    h.settled();
    assert_eq!(h.state::<Completed>().0, 1);
    assert_eq!(h.stuck(), 0);
}

//@show tasks: reporting without an actor reply
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Reports(pub u32);

#[reducer]
fn reports(this: &mut Reports, _: ()) {
    this.0 += 1;
}

pub struct SendReport(pub String);
pub struct ReportUpload;

struct Reporting {
    push: Push<Reports>,
    outgoing: tokio::sync::mpsc::UnboundedSender<String>,
}

actor! {
    Reporting {
        handlers { SendReport }
    }
}

#[handler]
fn send_report(In(this, SendReport(text), cx): In<Reporting, SendReport>) {
    this.push.send(());
    let outgoing = this.outgoing.clone();
    cx.task()
        .lane::<ReportUpload>()
        .latest()
        .detached()
        .spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = outgoing.send(text);
        });
}

feature! {
    pub ReportSending {
        exports { Reports }
    }
}

#[installs]
fn report_sending(
    cx: &Install<tokio::sync::mpsc::UnboundedSender<String>>,
) -> anyhow::Result<ReportSending> {
    let (reports, _) = cx.state::<Reports>().driven_by(|push| Reporting {
        push,
        outgoing: cx.params().clone(),
    });
    Ok(ReportSending(reports))
}
//@show-end

//@show tasks: only the newest detached report is sent
#[guinea::test]
fn a_new_detached_report_replaces_the_old_one_without_an_actor_reply(h: &mut Harness) {
    let (outgoing, mut received) = tokio::sync::mpsc::unbounded_channel();
    h.install::<ReportSending>(&outgoing).unwrap();
    let dispatch = h.dispatch::<Reports>();
    dispatch.emit(SendReport("old".into()));
    h.settled();
    h.advance(Duration::from_millis(500));
    dispatch.emit(SendReport("new".into()));
    h.settled();
    h.advance(Duration::from_millis(500));
    assert!(received.try_recv().is_err());
    h.advance(Duration::from_millis(500));
    assert_eq!(received.try_recv(), Ok("new".into()));
    assert!(received.try_recv().is_err());
    assert_eq!(h.state::<Reports>().0, 2);
}
//@show-end

//@show tasks: detached work is cancelled when its page leaves
#[guinea::test]
fn leaving_cancels_a_detached_report_before_it_is_sent(h: &mut Harness) {
    let (outgoing, mut received) = tokio::sync::mpsc::unbounded_channel();
    let page = h.child();
    page.install::<ReportSending>(&outgoing).unwrap();
    page.dispatch::<Reports>().emit(SendReport("report".into()));
    h.settled();
    page.leave();
    h.advance(Duration::from_secs(1));
    assert!(received.try_recv().is_err());
    assert_eq!(h.stuck(), 0);
}
//@show-end
