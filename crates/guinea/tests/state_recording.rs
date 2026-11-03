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

#[test]
fn detailed_diffs_replay_and_noops_keep_their_own_revision_and_cause() {
    let h = Harness::new(17);
    h.install::<Counter>(&()).unwrap();
    let recording = h.record_states(Config::default());
    recording.set_mode(Mode::Detailed);
    h.dispatch::<Count>().emit(Increase(3));
    h.settled();
    let reductions: Vec<_> = recording
        .take()
        .events
        .into_iter()
        .filter_map(|event| match event {
            Event::Reduced(entry) => Some(entry),
            _ => None,
        })
        .collect();
    assert_eq!(reductions.len(), 2);
    assert_ne!(reductions[0].id, reductions[1].id);
    assert_eq!(reductions[0].cause, reductions[1].cause);
    assert!(reductions[0].cause.is_some());
    let mut summary = Vec::new();
    for entry in reductions {
        let snapshots = entry.snapshots.unwrap();
        let (Snapshot::Json(before), Snapshot::Json(after), Snapshot::Json(patch)) =
            (snapshots.before, snapshots.after, snapshots.diff)
        else {
            panic!("expected captured JSON");
        };
        let mut replay = before.clone();
        let operations: json_patch::Patch = serde_json::from_value(patch.clone()).unwrap();
        json_patch::patch(&mut replay, &operations).unwrap();
        assert_eq!(replay, after);
        summary.push(
            serde_json::json!({"revision": entry.revision, "update": snapshots.update,
            "before": before, "after": after, "patch": patch}),
        );
    }
    insta::assert_debug_snapshot!("state_diff_replay", summary);
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

#[derive(Clone, Default, Serialize)]
struct Flag(bool);
#[reducer]
fn flag(this: &mut Flag, value: bool) {
    this.0 = value;
}

#[test]
fn diff_has_its_own_byte_limit_and_does_not_claim_missing_values_are_unchanged() {
    let tree = guinea_core::scope::ScopeTree::new();
    let recording = Recording::start(
        tree.scope(),
        Config {
            capacity: 16,
            snapshot_bytes: 5,
        },
    );
    recording.set_mode(Mode::Detailed);
    tree.push::<Flag>(true);
    let history = recording.take();
    let Event::Reduced(entry) = &history.events[1] else {
        panic!("expected reduction");
    };
    let snapshots = entry.snapshots.as_ref().unwrap();
    assert_eq!(snapshots.before, Snapshot::Json(false.into()));
    assert_eq!(snapshots.after, Snapshot::Json(true.into()));
    assert_eq!(snapshots.diff, Snapshot::LimitExceeded);
    tree.push::<Opaque>(());
    let history = recording.take();
    let Event::Reduced(entry) = &history.events[0] else {
        panic!("expected reduction");
    };
    assert_eq!(
        entry.snapshots.as_ref().unwrap().diff,
        Snapshot::Unavailable
    );
}

#[cfg(feature = "perfetto")]
#[test]
#[ignore = "requires official Trace Processor v59+ (TRACE_PROCESSOR)"]
fn perfetto_keeps_payloads_off_the_timeline_and_links_them_to_each_reduction() {
    let h = Harness::new(9);
    h.install::<Counter>(&()).unwrap();
    let sink = trace::sink_of(h.application_scope()).unwrap();
    let trace_file = trace::perfetto::Recording::start(&sink, Vec::new()).unwrap();
    let states = h.record_states(Config::default());
    h.dispatch::<Count>().emit(Increase(2));
    states.set_mode(Mode::Detailed);
    h.dispatch::<Count>().emit(Increase(3));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("states.pftrace");
    std::fs::write(&path, trace_file.finish().unwrap()).unwrap();
    let selection = include_str!("../../../tools/perfetto/sql/state-selection.sql.njk");
    let action_selection = selection.replace(
        "{{ event_id }}",
        "(SELECT MIN(slice_id) FROM guinea_state_causes)",
    );
    let single_selection = selection.replace(
        "{{ event_id }}",
        "(SELECT MAX(slice_id) FROM guinea_state_updates)",
    );
    let sql = format!(
        "{}\nCREATE PERFETTO TABLE action_updates AS {};\nCREATE PERFETTO TABLE single_update AS {};\n{}",
        include_str!("../../../tools/perfetto/sql/state-module.sql.njk"),
        action_selection.trim().trim_end_matches(';'),
        single_selection.trim().trim_end_matches(';'),
        r#"
      SELECT
        (SELECT COUNT(*) FROM guinea_state_updates) = 4
        AND (SELECT COUNT(*) FROM action_updates) = 2
        AND (SELECT COUNT(*) FROM single_update WHERE change = 'unchanged') = 1
        AND (SELECT COUNT(DISTINCT update_id) FROM guinea_state_updates) = 4
        AND (SELECT COUNT(*) FROM guinea_state_updates WHERE payload IS NULL AND change = 'not_captured') = 2
        AND (SELECT COUNT(*) FROM guinea_state_updates WHERE change = 'changed'
          AND JSON_EXTRACT(payload, '$.before.value.value') = 2
          AND JSON_EXTRACT(payload, '$.after.value.value') = 5
          AND JSON_EXTRACT(payload, '$.diff.value[0].op') = 'replace'
          AND JSON_EXTRACT(payload, '$.diff.value[0].path') = '/value') = 1
        AND (SELECT COUNT(*) FROM guinea_state_updates WHERE change = 'unchanged'
          AND JSON_ARRAY_LENGTH(JSON_EXTRACT(payload, '$.diff.value')) = 0) = 1
        AND (SELECT COUNT(*) FROM guinea_state_updates u JOIN slice s ON s.id = u.cause_slice_id
          WHERE EXTRACT_ARG(s.arg_set_id, 'debug.guinea_kind') = 'handle') = 4
        AND NOT EXISTS (SELECT 1 FROM slice WHERE EXTRACT_ARG(arg_set_id, 'debug.guinea_kind') = 'state.payload')
        AND NOT EXISTS (SELECT 1 FROM guinea_state_updates WHERE JSON_EXTRACT(payload, '$.before.value.secret') IS NOT NULL)
      AS valid;
    "#
    );
    let query = dir.path().join("states.sql");
    std::fs::write(&query, sql).unwrap();
    let result =
        std::process::Command::new(std::env::var_os("TRACE_PROCESSOR").expect("TRACE_PROCESSOR"))
            .args(["query", "--format", "csv", "-f"])
            .arg(query)
            .arg(path)
            .output()
            .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout)
            .unwrap()
            .trim()
            .replace('\r', ""),
        "\"valid\"\n1"
    );
}
