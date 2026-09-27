//! What `actor!` writes down about an actor, for devtools to draw.

use std::time::Duration;

use guinea_core::actor::shape::Channel;
use guinea_core::actor::{Cx, ManagedActor};
use guinea_macros::{Event, actor, handler};

pub struct Start;
pub struct Tick;
pub struct Report;

#[derive(Clone, Event)]
pub struct Stopped;

#[derive(Clone, Event)]
pub struct Finished;

#[derive(Debug, Default)]
pub struct Poller;

actor! {
    Poller {
        handlers {
            Start => { bg Tick },
            Tick => { bg Tick loop, send Report },
            Report,
            Stopped,
        }
        publishes { Finished }
    }
}

#[handler]
fn start(_this: &mut Poller, _: Start, cx: Cx) {
    cx.spawn_bg::<Tick, _>(async {
        tokio::time::sleep(Duration::from_millis(1)).await;
        Tick
    });
}

#[handler]
fn tick(_this: &mut Poller, _: Tick) {}

#[handler]
fn report(_this: &mut Poller, _: Report) {}

#[handler]
fn stopped(_this: &mut Poller, _: Stopped) {}

#[test]
fn an_actor_knows_where_it_was_declared() {
    let declared = Poller::SHAPE.declared.expect("actor! records its place");
    assert!(declared.file.ends_with("shape.rs"), "{declared:?}");
    assert_eq!(declared.line, 23, "the line of the actor's name");
    let path = declared.path().expect("the sources are here");
    assert!(path.ends_with("tests/shape.rs") || path.ends_with("tests\\shape.rs"));
    assert!(std::fs::read_to_string(path)
        .expect("readable")
        .lines()
        .nth(declared.line as usize - 1)
        .is_some_and(|line| line.trim_start().starts_with("Poller")));
}

#[test]
fn a_handler_knows_where_it_was_written() {
    let handles = Poller::SHAPE.handles;
    let start = handles[0].declared.expect("#[handler] records its place");
    let path = start.path().expect("the sources are here");
    let line = std::fs::read_to_string(path)
        .expect("readable")
        .lines()
        .nth(start.line as usize - 1)
        .map(str::to_string);
    assert!(
        line.as_deref().is_some_and(|line| line.starts_with("fn start(")),
        "{line:?}"
    );
}

#[test]
fn the_declared_edges_are_there_to_read() {
    let shape = Poller::SHAPE;

    let handled: Vec<&str> = shape.handles.iter().map(|h| (h.message)()).collect();
    assert_eq!(handled.len(), 4);
    assert!(handled[0].ends_with("Start"), "{handled:?}");

    let tick = &shape.handles[1];
    let edges = tick.edges.expect("Tick declared its edges");
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0].channel, Channel::Bg);
    assert!(edges[0].looping);
    assert!((edges[0].target)().ends_with("Tick"));
    assert_eq!(edges[1].channel, Channel::Send);
    assert!(!edges[1].looping);
    assert!((edges[1].target)().ends_with("Report"));

    assert!(shape.handles[2].edges.is_none(), "Report declared nothing");

    let published: Vec<&str> = shape.publishes.iter().map(|name| name()).collect();
    assert_eq!(published.len(), 1);
    assert!(published[0].ends_with("Finished"));
    assert!(shape.subscribes.is_empty());
}
