---
title: Features and state
sidebar:
  order: 3
---

A page needs a task list. It shows the tasks and asks to add or clear them;
the rules for accepting a task belong to a feature. The same feature can be
used from a different page, a different toolkit, or a test without a screen.

We will build `TaskList`: the state a page reads, the actions it sends, and
the code that handles them. None of this depends on a backend.

## State and updates

`Tasks` is what the page can read. The reducer describes how an update changes
it:

<!-- shown: features: state and updates -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Tasks(pub Vec<String>);

#[derive(Clone)]
pub enum Change {
    Added(String),
    Cleared,
}

#[reducer]
fn tasks(this: &mut Tasks, change: Change) {
    match change {
        Change::Added(title) => this.0.push(title),
        Change::Cleared => this.0.clear(),
    }
}
```
<!-- /shown -->

`#[reducer]` implements `Reducer` for `Tasks`, with `Change` as its update
type. The state starts at `Default`: an empty list. An `Added` update appends
a title; `Cleared` removes the tasks.

An update describes a change already chosen by the feature. It is different
from an action, which asks the feature to do something.

## What a page asks for

The page sends one of two actions:

<!-- shown: features: actions -->
```rust
pub struct AddTask(pub String);
pub struct ClearTasks;
```
<!-- /shown -->

`AddTask` carries what the user typed. It may contain surrounding whitespace
or no title at all. The page can send it without knowing how the feature
handles those cases.

The page uses actions rather than constructing `Change` updates. Reading
state does not hand it a `Push`: the write handle is given to the feature
during installation.

## An actor decides

The actor handles actions and sends updates through `Push<Tasks>`:

<!-- shown: features: an actor decides -->
```rust
#[derive(Debug)]
struct Editing {
    push: Push<Tasks>,
}

actor! {
    Editing {
        handlers { AddTask, ClearTasks }
    }
}

#[handler]
fn add(In(this, AddTask(title), _): In<Editing, AddTask>) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }

    this.push.send(Change::Added(title.to_owned()));
}

#[handler]
fn clear(In(this, _, _): In<Editing, ClearTasks>) {
    this.push.send(Change::Cleared);
}
```
<!-- /shown -->

The `actor!` declaration lists the messages `Editing` handles. Each
`#[handler]` implements the corresponding handler; `In` carries the actor,
the message and its context. These handlers do not need the context, so they
ignore the third field.

Adding a task takes two steps: choose a valid title, then send the update.
Whitespace is trimmed and an empty title produces no update. Clearing needs
no preparation and sends `Change::Cleared` directly.

`Editing` is private. The page uses the state and action types without
referring to the actor by name or address. The feature's implementation
can change while the page's code stays the same.

## Installing the feature

The declaration says what the feature exports. The installation function
creates its state and actor:

<!-- shown: features: installation -->
```rust
feature! {
    pub TaskList {
        exports { Tasks }
    }
}

#[installs]
fn task_list(cx: &Install) -> anyhow::Result<TaskList> {
    let (tasks, _) = cx.state::<Tasks>().driven_by(|push| Editing { push });
    Ok(TaskList(tasks))
}
```
<!-- /shown -->

`cx.state::<Tasks>()` registers the state as belonging to this feature.
`driven_by` creates the actor, passing `Push<Tasks>` to its constructor.
It returns a `Bound<Tasks>` handle and the actor's address. This installation
does not use the address: `Bound` is enough to export the state.

Here `feature!` creates a tuple struct `TaskList` with one `Bound<Tasks>`
field. The function returns that handle in `TaskList(tasks)`, and feature
installation publishes the declared export. `Tasks` then becomes readable
in the installation scope and its descendants.

The `TaskList` value owns neither the actor nor the state cell; the scope
owns both. A page can therefore install the feature without keeping the
returned value.

A page installs it with `cx.install::<TaskList>(&())?`, then reads
`cx.read::<Tasks>()`. That read returns the state and a `Dispatch` for the
feature that owns it. The page sends `AddTask` or `ClearTasks` through that
dispatcher. The [pages chapter](../pages/) shows how reads and widget events
fit into each backend.

## Testing the rules

The harness installs the same feature and sends the same actions as a page:

<!-- shown: features: actions become state -->
```rust
#[guinea::test]
fn a_task_gets_a_title_before_it_is_added(h: &mut Harness) {
    h.install::<TaskList>(&()).unwrap();
    let dispatch = h.dispatch::<Tasks>();

    dispatch.emit(AddTask("  Read the book  ".into()));
    dispatch.emit(AddTask("   ".into()));
    h.settled();

    assert_eq!(h.state::<Tasks>().0, vec!["Read the book"]);

    dispatch.emit(ClearTasks);
    h.settled();

    assert!(h.state::<Tasks>().0.is_empty());
}
```
<!-- /shown -->

`h.settled()` runs the work queued by the actions before the assertions read
the state. The test checks the feature's behavior: titles are trimmed, blank
tasks are rejected, and clearing empties the list.

The dispatcher belongs to the feature that owns the state it was obtained
for. Two instances in different scopes can therefore answer the same actions
independently:

<!-- shown: features: separate instances -->
```rust
#[guinea::test]
fn two_pages_can_have_separate_task_lists(h: &mut Harness) {
    let work = h.child();
    let home = h.child();
    work.install::<TaskList>(&()).unwrap();
    home.install::<TaskList>(&()).unwrap();

    work.dispatch::<Tasks>()
        .emit(AddTask("Review a change".into()));
    home.dispatch::<Tasks>().emit(AddTask("Buy bread".into()));
    h.settled();

    assert_eq!(work.state::<Tasks>().0, vec!["Review a change"]);
    assert_eq!(home.state::<Tasks>().0, vec!["Buy bread"]);
}
```
<!-- /shown -->

If both pages should share one list instead, install `TaskList` in their
common layout. Both reads then reach that instance, and both dispatchers
send actions to it. [Scopes](../scopes/) explains how placement determines
sharing and lifetime.

## Asynchronous values: `Load<T>`

An empty list and a list that has not arrived yet are different states.
Use `Load<T>` from `guinea::prelude::*` when a value arrives asynchronously:

| Variant | Meaning |
| --- | --- |
| `NotAsked` | No load has been requested yet. |
| `Loading` | Waiting for a value. |
| `Ready(T)` | A value is available, including an empty collection. |
| `Failed(Arc<anyhow::Error>)` | Loading failed. |

**`Load::default()` is `Loading`, not `NotAsked`.** The type stores status;
it does not start a task or make transitions automatically. A feature that
waits for an explicit user action can choose `NotAsked` in its state:

<!-- shown: loading: state -->
```rust
#[derive(Clone, PartialEq, Debug)]
pub struct Listing {
    pub rows: Load<Vec<String>>,
}

impl Default for Listing {
    fn default() -> Self {
        Self {
            rows: Load::NotAsked,
        }
    }
}

#[reducer]
fn listing(this: &mut Listing, rows: Load<Vec<String>>) {
    this.rows = rows;
}
```
<!-- /shown -->

The reducer replaces the loading state with the update it receives.
The actor first sends `Loading`, then starts work and converts its result
into `Ready` or `Failed`:

<!-- shown: loading: a loader -->
```rust
pub struct Refresh {
    pub simulate_failure: bool,
}

async fn fetch(simulate_failure: bool) -> anyhow::Result<Vec<String>> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    anyhow::ensure!(!simulate_failure, "service unavailable");
    Ok(vec!["guinea".into(), "shown".into()])
}

#[derive(Debug)]
struct Loader {
    push: Push<Listing>,
}

actor! {
    Loader {
        handlers { Refresh }
    }
}

#[handler]
fn refresh(In(this, Refresh { simulate_failure }, cx): In<Loader, Refresh>) {
    this.push.send(Load::Loading);
    cx.task().latest().pushing(this.push).spawn(async move {
        match fetch(simulate_failure).await {
            Ok(rows) => Load::Ready(rows),
            Err(error) => Load::failed(error),
        }
    });
}

feature! {
    pub Catalogue {
        exports { Listing }
    }
}

#[installs]
fn catalogue(cx: &Install) -> anyhow::Result<Catalogue> {
    let (listing, _) = cx.state::<Listing>().driven_by(|push| Loader { push });
    Ok(Catalogue(listing))
}
```
<!-- /shown -->

`fetch` is a testable stand-in for a service call: after one second it
returns two names or the requested simulated failure. It does not contact
a server. This example uses `Duration` from `std::time` and Tokio's `time`
feature, as in [background tasks](../tasks/).

`Load::failed(error)` wraps an error in an `Arc`, making the failed state
cheap to clone. `latest` handles overlapping requests; `Load` itself does
not know which request is newer. Without that policy or another explicit
check, an old failure could replace a newer result.

### Presenting the state

A page can match all four variants and choose the appropriate widgets.
This backend-independent helper produces text so the same decisions can
be tested without a window:

<!-- shown: loading: presenting each state -->
```rust
fn status(listing: &Listing) -> String {
    match &listing.rows {
        Load::NotAsked => "Load the catalogue".into(),
        Load::Loading => "Loading…".into(),
        Load::Ready(rows) => format!("{} items", rows.len()),
        Load::Failed(error) => format!("Could not load: {error}"),
    }
}
```
<!-- /shown -->

<!-- shown: loading: successful loading -->
```rust
#[guinea::test]
fn a_request_moves_from_not_asked_through_loading_to_ready(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    assert!(matches!(h.state::<Listing>().rows, Load::NotAsked));
    assert_eq!(status(&h.state::<Listing>()), "Load the catalogue");

    h.dispatch::<Listing>().emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    assert!(h.state::<Listing>().rows.is_loading());
    assert_eq!(status(&h.state::<Listing>()), "Loading…");

    h.advance(Duration::from_secs(1));
    assert_eq!(
        h.state::<Listing>().rows.ready().unwrap(),
        &["guinea", "shown"]
    );
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}
```
<!-- /shown -->

`ready()` returns `Some(&T)` only for `Ready`. `is_loading()` tests
`Loading`, and `error()` borrows the error only for `Failed`. An empty
`Ready(Vec::new())` is still a successful result, not a loading indicator.

### Failure and retry

A retry is another `Refresh`: the actor sends `Loading` again and can
replace the failure with a successful result.

<!-- shown: loading: failure and retry -->
```rust
#[guinea::test]
fn a_failed_load_can_be_retried(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    let dispatch = h.dispatch::<Listing>();
    dispatch.emit(Refresh {
        simulate_failure: true,
    });
    h.settled();
    h.advance(Duration::from_secs(1));

    let failed = h.state::<Listing>();
    assert!(failed.rows.ready().is_none());
    assert_eq!(
        failed.rows.error().unwrap().to_string(),
        "service unavailable"
    );
    assert_eq!(status(&failed), "Could not load: service unavailable");

    dispatch.emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    assert!(h.state::<Listing>().rows.is_loading());
    assert!(h.state::<Listing>().rows.error().is_none());
    h.advance(Duration::from_secs(1));
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}
```
<!-- /shown -->

`Failed` values compare by the identity of their error `Arc`, not by their
error text. Cloning a failed value preserves equality; constructing another
failure with the same text produces a different value. `Ready` compares
its contained data when `T` supports equality.

In this example, refreshing a ready list replaces it with `Loading`, so
the old rows are no longer in the state. To keep previous data visible
while refreshing, model that separately, for example with a last-successful
value and a request status. `Load` does not retain an earlier value for you.

## What belongs here

The feature holds the accepted tasks. Text still being entered, the selected
row and a local filter can stay in the page's fields. If other screens need
those values, or they must outlive the page, they can belong to a feature
with the appropriate lifetime.

An actor is useful when actions need decisions, several handlers share
working state, or background work returns messages. It is not required for
every reducer: `cx.state::<Tasks>().plain()` returns a `Bound<Tasks>` without
creating an actor. Its `push` method sends updates directly to the reducer.
For example, installation can register a subscription that transforms
incoming values and updates the state through that handle.
