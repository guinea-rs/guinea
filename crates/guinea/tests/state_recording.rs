use std::cell::{Cell, RefCell};
use std::rc::Rc;

use guinea::app::Harness;
use guinea::observability::state::{Config, Event, Mode, Recording, Snapshot};
use guinea::observability::trace::{self, Point, Trace};
use guinea::prelude::*;
use serde::{Serialize, Serializer};

thread_local! {
    static SERIALIZATIONS: Cell<usize> = const { Cell::new(0) };
}

fn capture_value<S: Serializer>(value: &i32, serializer: S) -> Result<S::Ok, S::Error> {
    SERIALIZATIONS.set(SERIALIZATIONS.get() + 1);
    value.serialize(serializer)
}

#[derive(Clone, Serialize)]
struct Count {
    #[serde(serialize_with = "capture_value")]
    value: i32,
    #[serde(skip)]
    secret: String,
}

impl Default for Count {
    fn default() -> Self {
        Self {
            value: 0,
            secret: "not recorded".into(),
        }
    }
}

#[derive(Clone, Serialize)]
enum Update {
    Add(i32),
}

#[reducer]
fn count(this: &mut Count, Update::Add(by): Update) {
    this.value += by;
}

struct Increase(i32);
struct Later;

#[derive(Debug)]
struct Worker {
    push: Push<Count>,
}

actor! {
    Worker {
        handlers { Increase, Later => { task Update } Update }
    }
}

#[handler]
fn increase(In(this, Increase(by), _): In<Worker, Increase>) {
    this.push.send(Update::Add(by));
    this.push.send(Update::Add(0));
}

#[handler]
fn later(In(this, _, cx): In<Worker, Later>) {
    cx.task()
        .pushing(this.push)
        .spawn(async { Update::Add(10) });
}

#[handler]
fn update(In(this, update, _): In<Worker, Update>) {
    this.push.send(update);
}

feature! { Counter { exports { Count } } }

#[installs]
fn counter(cx: &Install) -> anyhow::Result<Counter> {
    let (state, _) = cx.state::<Count>().driven_by(|push| Worker { push });
    Ok(Counter(state))
}

fn summary(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            Event::ModeChanged { from, to, .. } => format!("mode {from:?} -> {to:?}"),
            Event::Reduced(entry) => {
                let mut text = format!("revision {}", entry.revision);
                if let Some(snapshots) = &entry.snapshots {
                    text.push_str(&format!(
                        " {:?}: {:?} -> {:?}",
                        snapshots.update, snapshots.before, snapshots.after
                    ));
                } else {
                    text.push_str(" (metadata only)");
                }
                text
            }
        })
        .collect()
}

#[test]
fn harness_records_every_reduction_and_switches_capture_for_the_whole_app() {
    let h = Harness::new(17);
    h.install::<Counter>(&()).unwrap();
    let scope = h.segment().scope();
    let recording = h.record_states(Config::default());
    let seen = Rc::new(RefCell::new(Vec::new()));
    let collect = seen.clone();
    let _observer = trace::sink_of(scope).unwrap().observe(move |event| {
        if let Trace::Begin(record) | Trace::Mark(record) = event {
            collect.borrow_mut().push(record.clone());
        }
    });

    SERIALIZATIONS.set(0);
    h.dispatch::<Count>().emit(Increase(2));
    h.settled();
    assert_eq!(SERIALIZATIONS.get(), 0);
    let normal = recording.take();
    assert_eq!(normal.events.len(), 2);
    for event in &normal.events {
        let Event::Reduced(entry) = event else {
            panic!("expected reduction")
        };
        assert_eq!(entry.scope, scope.id());
        assert_eq!(entry.reducer, std::any::type_name::<Count>());
        assert_eq!(entry.update, std::any::type_name::<Update>());
        let cause = entry.cause.expect("the handler is the cause");
        assert!(
            seen.borrow()
                .iter()
                .any(|record| record.id == cause && matches!(record.point, Point::Handle { .. }))
        );
        assert!(
            seen.borrow()
                .iter()
                .any(|record| record.parent == Some(cause)
                    && matches!(
                        &record.point,
                        Point::Plugin {
                            kind: "state.update",
                            ..
                        }
                    ))
        );
    }

    recording.set_mode(Mode::Detailed);
    recording.set_mode(Mode::Detailed);
    h.dispatch::<Count>().emit(Increase(3));
    h.settled();
    assert_eq!(SERIALIZATIONS.get(), 4);
    let detailed = recording.take();
    let Event::Reduced(entry) = &detailed.events[1] else {
        panic!("expected reduction")
    };
    let Snapshot::Json(before) = &entry.snapshots.as_ref().unwrap().before else {
        panic!("expected JSON")
    };
    assert!(before.get("secret").is_none());

    let child = h.child();
    child.install::<Counter>(&()).unwrap();
    child.dispatch::<Count>().emit(Increase(7));
    h.settled();
    let children = recording.take();
    for event in &children.events {
        let Event::Reduced(entry) = event else {
            panic!("expected reduction")
        };
        assert_eq!(entry.scope, child.scope().id());
        assert!(entry.snapshots.is_some());
    }
    assert_eq!(children.events.len(), 2);

    recording.set_mode(Mode::Normal);
    let captured = SERIALIZATIONS.get();
    h.dispatch::<Count>().emit(Increase(1));
    h.settled();
    assert_eq!(SERIALIZATIONS.get(), captured);
    let again = recording.take();
    assert_eq!(h.state::<Count>().value, 6);
    assert_eq!(h.state::<Count>().secret, "not recorded");

    let all = [
        summary(&normal.events),
        summary(&detailed.events),
        summary(&again.events),
    ]
    .concat();
    insta::assert_debug_snapshot!(all);
}

#[derive(Clone, Default)]
struct Opaque;
#[reducer]
fn opaque(_: &mut Opaque, _: ()) {}

#[derive(Clone, Default)]
struct Broken;
impl Serialize for Broken {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("deliberate capture failure"))
    }
}
#[reducer]
fn broken(_: &mut Broken, _: ()) {}

#[test]
fn budgets_missing_serializers_and_failed_serializers_are_explicit() {
    let tree = guinea_core::scope::ScopeTree::new();
    tree.push::<Count>(Update::Add(1));
    let recording = Recording::start(
        tree.scope(),
        Config {
            capacity: 2,
            snapshot_bytes: 4,
        },
    );
    recording.set_mode(Mode::Detailed);
    tree.push::<Count>(Update::Add(2));
    tree.push::<Count>(Update::Add(3));
    let history = recording.take();
    assert_eq!(history.dropped, 1);
    assert_eq!(history.events.len(), 2);
    let Event::Reduced(entry) = &history.events[1] else {
        panic!("expected reduction")
    };
    assert_eq!(entry.revision, 3);
    assert_eq!(
        entry.snapshots.as_ref().unwrap().before,
        Snapshot::LimitExceeded
    );
    assert_eq!(
        entry.snapshots.as_ref().unwrap().update,
        Snapshot::LimitExceeded
    );
    tree.push::<Opaque>(());
    tree.push::<Broken>(());
    let history = recording.take();
    let captures: Vec<_> = history
        .events
        .into_iter()
        .map(|event| {
            let Event::Reduced(entry) = event else {
                panic!("expected reduction")
            };
            entry.snapshots.unwrap()
        })
        .collect();
    assert_eq!(captures[0].before, Snapshot::Unavailable);
    assert_eq!(captures[0].update, Snapshot::Json(().into()));
    // A larger budget distinguishes serializer errors from byte limits.
    drop(recording);
    let recording = Recording::start(tree.scope(), Config::default());
    recording.set_mode(Mode::Detailed);
    tree.push::<Broken>(());
    let history = recording.take();
    let Event::Reduced(entry) = &history.events[1] else {
        panic!("expected reduction")
    };
    assert_eq!(
        entry.snapshots.as_ref().unwrap().before,
        Snapshot::Failed("deliberate capture failure".into())
    );
}

#[test]
fn async_replies_keep_the_cause_and_recorders_do_not_cross_application_trees() {
    let h = Harness::new(33);
    h.install::<Counter>(&()).unwrap();
    let recording = h.record_states(Config::default());
    recording.set_mode(Mode::Detailed);
    h.dispatch::<Count>().emit(Later);
    h.settled();
    let history = recording.take();
    let Event::Reduced(entry) = &history.events[1] else {
        panic!("expected reduction")
    };
    assert!(entry.cause.is_some());
    assert_eq!(h.state::<Count>().value, 10);

    let other = guinea_core::scope::ScopeTree::new();
    let independent = Recording::start(other.scope(), Config::default());
    other.push::<Count>(Update::Add(5));
    let history = independent.take();
    let Event::Reduced(entry) = &history.events[0] else {
        panic!("expected reduction")
    };
    assert!(entry.snapshots.is_none());
    assert!(recording.take().events.is_empty());
    independent.set_mode(Mode::Detailed);
    drop(independent);
    SERIALIZATIONS.set(0);
    other.push::<Count>(Update::Add(1));
    assert_eq!(SERIALIZATIONS.get(), 0);
}

#[test]
fn recorded_values_survive_scope_removal_without_retaining_the_scope_or_state() {
    let tree = guinea_core::scope::ScopeTree::new();
    let recording = Recording::start(tree.scope(), Config::default());
    recording.set_mode(Mode::Detailed);
    let child = tree.child();
    child.push::<Count>(Update::Add(9));
    let state = child.state::<Count>();
    let weak = Rc::downgrade(&*state.borrow());
    drop(state);
    child.remove();
    assert!(!child.is_alive());
    assert!(weak.upgrade().is_none());
    drop(tree);
    let history = recording.take();
    let Event::Reduced(entry) = &history.events[1] else {
        panic!("expected reduction")
    };
    assert_eq!(entry.scope, child.id());
    let Snapshot::Json(after) = &entry.snapshots.as_ref().unwrap().after else {
        panic!("expected recorded value")
    };
    assert_eq!(after["value"], 9);
}
