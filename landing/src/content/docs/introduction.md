---
title: Introduction
sidebar:
  order: 1
---

Rust has good toolkits and no application layer above them: a toolkit draws
the screen, and how the application around it is built is left to each
project. The answers differ, but every application past a demo meets the same
problems:

- messages forwarded through layers to where they are handled;
- request ids, so a late answer does not overwrite a newer one;
- setup, teardown and refetching written by hand for every screen;
- one huge state everything reaches into, and no tests for it;
- settings saved and synced by hand.

## What guinea is

MVI for a whole application:

- **Intent** - a page emits what the user asked for, and changes only its own
  view state;
- **Model** - a reducer, the one place a state changes, driven by an actor that
  decides;
- **View** - the page, written the way its toolkit works: WinUI, iced, egui,
  ratatui or Slint.

A click goes around one loop:

```mermaid
flowchart LR
    subgraph View
        page[page]
    end
    subgraph Model
        actor[actor]
        task[background task]
        reducer[reducer]
    end
    page -->|"action (Intent)"| actor
    actor -->|spawns| task
    task -->|answer| actor
    actor -->|push| reducer
    reducer -->|state| page
```

1. The page emits an action - what the user asked for, not what to change.
2. The actor of the feature that owns the state decides what it means. Slow
   work - a request, a file - runs as a background task, and its answer comes
   back to the actor as one more message.
3. The actor pushes an update to the reducer, the one place the state changes.
4. The page reads the state and draws again.

What only the page cares about - which tab is open, what is typed in a filter -
does not go around the loop: the page keeps it itself.

The application is built out of pages. Everything lives in a scope - the
application's, a window's, a layout's, a page's - and goes with it: what a page
set up ends when the user leaves it, with nothing to tear down by hand.

State lives with the screen that needs it, and is shared by installing it
higher up - in a layout, or in the application. A page reads what it installed
itself, what a layout above it exports, and what the application provides;
anything else is not in reach, and a test sets up exactly that and no more.

## A first look

A counter: the state, the reducer that is the only place it changes, the actor
that decides what goes into it, and the feature that installs the two.

<!-- shown: introduction: a feature -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

#[reducer]
fn count(this: &mut Count, by: u32) {
    this.0 += by;
}

pub struct Add(pub u32);

#[derive(Debug)]
pub struct Counting {
    push: Push<Count>,
}

actor! {
    Counting {
        handlers { Add }
    }
}

#[handler]
fn add(In(this, Add(by), _): In<Counting, Add>) {
    this.push.send(by);
}

feature! {
    pub Counter {
        exports { Count }
    }
}

#[installs]
fn counter(cx: &Install) -> anyhow::Result<Counter> {
    let (count, _) = cx.state::<Count>().driven_by(|push| Counting { push });
    Ok(Counter(count))
}
```
<!-- /shown -->

A test installs the feature with nothing around it, sends what a page would,
and reads what came out. No window, no event loop.

<!-- shown: introduction: a test -->
```rust
#[guinea::test]
fn what_is_added_is_counted(h: &mut Harness) {
    h.install::<Counter>(&()).unwrap();

    h.dispatch::<Count>().emit(Add(2));
    h.dispatch::<Count>().emit(Add(3));
    h.settled();

    assert_eq!(h.state::<Count>().0, 5);
}
```
<!-- /shown -->
