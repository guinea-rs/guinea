---
title: Background tasks
sidebar:
  order: 5
---

A handler runs on the UI thread. It can decide what to do and update state,
but waiting for a network response or doing expensive work there would
block the interface. Start asynchronous work with `cx.task()`. Its result
can go straight to a reducer or come back to the actor as a message.

There are two separate questions: what should happen to a result, and what
should happen when another request arrives before the first finishes?
This chapter builds a search feature that shows only the newest query's
answer. The code is independent of the GUI backend.

## The request and its answer

The page sends `Query`; the background task returns `Found`. The reducer
stores the search result:

<!-- shown: tasks: state and messages -->
```rust
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
```
<!-- /shown -->

To keep the example self-contained, `search` waits one second and filters a
fixed list. It does not access a network. A real implementation can await a
client library in its place.

<!-- shown: tasks: simulated search -->
```rust
async fn search(query: String) -> Found {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let names = ["guinea", "guinea-app", "shown"]
        .into_iter()
        .filter(|name| name.contains(query.as_str()))
        .map(str::to_owned)
        .collect();
    Found { query, names }
}
```
<!-- /shown -->

This helper uses Tokio's `time` feature. With guinea's default `own-runtime`
feature, the backend starts an async runtime when needed. If you disable
that feature, enter your own runtime before starting the application.

## Starting work from a handler

<!-- shown: tasks: an actor starts work -->
```rust
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
```
<!-- /shown -->

`pushing(this.push)` sends the completed result straight to the `Results`
reducer. `search` returns `Found`, which is already its update type, so
there is no need for a `Found` handler or a `task Found` declaration.

The builder returned by `cx.task()` starts nothing until `spawn` is called.
The future and its output must be `Send + 'static`: move owned request data
into the future, not a borrow of the actor. The actor itself and its handlers
stay on the UI thread. guinea applies the completed update there.

Install the actor and its state as an ordinary feature:

<!-- shown: tasks: installation -->
```rust
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
```
<!-- /shown -->

The page reads `Results` and emits `Query` through its dispatcher. It does
not need to own a task handle or forward completed results itself.

## Choosing what overlaps

Without a policy, every spawned task runs independently. All completed
answers are delivered, in completion order. That is useful for independent
operations, but wrong for a search box: a slow answer for an old query could
replace the current results.

`latest()` replaces work already in the same **lane**. The old future is
cancelled, and an old answer already on its way to the UI thread is also
discarded if newer work has replaced it before delivery.

The available policies are:

| Policy | When new work arrives while the lane is busy |
| --- | --- |
| No policy | Run it independently; deliver every answer. |
| `latest()` | Cancel the old work and start the new work. |
| `conflate()` | Finish the running work, then run only the newest pending request. |
| `exhaust()` | Ignore the new request until the running work finishes. |

For example, `conflate` can reduce repeated requests to refresh a snapshot;
`exhaust` can ignore repeated clicks while an operation is already running.
Neither is a queue that preserves every request.

A lane belongs to one actor instance. By default its identity is the future's
output type: `Found` here. No explicit name is needed. Use `lane::<K>()`
to separate independent operations that happen to return the same type.
`keyed(value)` further separates work by an equal-comparable key, such as
a document ID. A lane keeps its first policy for the actor's lifetime;
trying to spawn with another policy in that lane panics.

## Testing replacement without real waiting

<!-- shown: tasks: only the newest query answers -->
```rust
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
```
<!-- /shown -->

`h.settled()` runs ready work without advancing time. `h.advance()` moves the
paused Tokio clock and runs what becomes ready. The test starts the second
query halfway through the first. At the first query's deadline there is
still no result; only the replacement is allowed to answer.

## Cancelling explicitly

A cancel button can send `CancelSearch`. Its handler names the lane to stop:

<!-- shown: tasks: cancelling a lane -->
```rust
#[handler]
fn cancel_search(In(_, _, cx): In<Searching, CancelSearch>) {
    cx.task().lane::<Found>().cancel();
}
```
<!-- /shown -->

`cancel()` cancels running work and drops pending work in that lane. With
`keyed(value)`, it cancels only that key; without a key, it cancels every key
in the lane. It must be given an explicit lane, even when that lane is named
by the future's output type. Here `lane::<Found>()` identifies the same
lane that `spawn(search(text))` selects automatically.

Cancellation does not dispose the actor or permanently close the lane:

<!-- shown: tasks: an explicit cancellation -->
```rust
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
```
<!-- /shown -->

## Leaving the page

The actor's lifetime also limits its tasks. Removing the page's scope
disposes the actor, cancels its work and prevents further answers:

<!-- shown: tasks: leaving cancels the work -->
```rust
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
```
<!-- /shown -->

`h.stuck()` counts tasks still waiting. Here it confirms that a pending
search does not survive the page. If the search must survive navigation,
install its feature in a common layout or in the application instead.
Keeping a page asleep is not the same as removing its scope.

## Equivalent: replying through the actor

Without `pushing`, the task returns its result to the actor as a message.
For this example, forwarding that message to the reducer has the same
effect:

<!-- shown: tasks: replying through the actor -->
```rust
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
```
<!-- /shown -->

This replaces the actor declaration and handlers above; it is not a second
`Query` handler for the same actor. `task Found` declares the reply, and
the `Found` handler forwards it to state. Installation and cancellation
stay the same, and both variants use `Found` as the default lane.

Use this form when the actor needs to inspect a result, update its own
working fields or choose another action before changing state. When the
handler only forwards the update, `pushing` avoids that extra step.
If all you need is to convert the result into an update on the UI thread,
use `pushing_with`; keep that conversion cheap.

## What cancellation does not do

Ordinary `spawn` cancellation drops the future at an await boundary. It
does not undo a server request already accepted, a file already written,
or synchronous code that is currently blocking a worker thread. Keep
blocking operations out of UI handlers and use the appropriate blocking
API of your async runtime where needed.

## Finishing asynchronous cleanup

Suppose a task uses a session that must be closed with a request and an
acknowledgement. Dropping the future is not enough to perform that protocol:
it needs to await the acknowledgement after stopping its main work.

Here is a small session model using two one-shot channels, with no network.
`close` sends the request and waits for confirmation. `session_pair` gives
the test the other ends, so it can act as the peer:

<!-- shown: tasks: a session closes asynchronously -->
```rust
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
```
<!-- /shown -->

`spawn_with` hands a cancellation token to the function that creates the
future. Unlike `spawn`, it does not drop that future when cancellation
arrives. The future must observe the token and finish itself:

<!-- shown: tasks: cooperative work -->
```rust
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
```
<!-- /shown -->

The one-second sleep stands in for the session's main operation.
`cancel.guard(...)` ends that operation on cancellation, returning `None`;
on normal completion it returns `Some(())`. Either way, the future then
awaits `session.close()`. That cleanup is deliberately **outside** the
cancellation guard, or it could be interrupted by the same cancellation.

`Completed` counts successful results accepted by the reducer, not closed
sessions. When cancelled, the future still closes the session and returns
`SessionFinished`, but guinea discards that answer:

<!-- shown: tasks: cancellation lets cleanup finish but drops the answer -->
```rust
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
```
<!-- /shown -->

The first `settled` confirms that the running operation has not requested
closure. After `StopSession`, the peer receives the close request, but the
task is still waiting for confirmation: `h.stuck()` is `1`. Sending the
confirmation lets cleanup finish. The task is gone, while `Completed`
remains `0`. Without cancellation, the same cleanup completes before the
result is delivered and the counter increases.

Removing the page also signals cancellation rather than dropping this
cooperative future. It can finish closing the session after the page is gone:

<!-- shown: tasks: leaving also allows cooperative cleanup -->
```rust
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
```
<!-- /shown -->

The harness is still running in this test. This is not a guarantee that
application shutdown waits for cleanup: the async runtime must remain alive
to execute it. If the peer never confirms closure, this example keeps
waiting; a real protocol should choose its timeout and failure handling.
Likewise, a `spawn_with` future that ignores its token does not stop just
because cancellation was requested.

## Work without a reply

Suppose a page sends a diagnostic report. The UI does not need the outcome:
the task should perform the operation and finish without sending another
message to the actor. Use `detached()` for this. Its future returns `()`;
there is no reply handler to implement.

Here a channel stands in for the service receiving reports, and the
one-second wait simulates sending latency. `Reports` counts send requests,
not service replies:

<!-- shown: tasks: reporting without an actor reply -->
```rust
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
```
<!-- /shown -->

If the user requests another report before the previous one is sent,
`latest()` cancels the old send. `ReportUpload` is an empty marker type naming
this group of tasks. `.lane::<ReportUpload>()` is required here: there is no
reply type for guinea to use as the lane's name. Give a different group a
different marker so they do not cancel each other's work.

<!-- shown: tasks: only the newest detached report is sent -->
```rust
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
```
<!-- /shown -->

“No reply” does not mean “continue after the page closes.” The actor still
owns the task: removing its scope cancels an unfinished send:

<!-- shown: tasks: detached work is cancelled when its page leaves -->
```rust
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
```
<!-- /shown -->

For work that must survive the page, install its actor in a longer-lived
scope, such as the application's. `detached()` does not change its lifetime.
