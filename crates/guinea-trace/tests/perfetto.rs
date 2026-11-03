#![cfg(feature = "perfetto")]

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use guinea_trace::perfetto::Recording;
use guinea_trace::{Point, Sink, Trace, enter, mark, mark_under, record_into};
use perfetto_protos::trace::Trace as ProtoTrace;
use perfetto_protos::track_event::track_event::Type;
use protobuf::Message;

#[test]
fn native_packets_keep_cross_thread_causes_counters_and_wall_intervals() {
    let sink = Sink::new();
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    let _current = record_into(&sink);
    let action = enter(|| Point::Action { message: "Refresh" });
    let cause = action.id();
    let elsewhere = sink.clone();
    let worker = std::thread::Builder::new()
        .name("scan-worker".into())
        .spawn(move || {
            let _current = record_into(&elsewhere);
            mark_under(Some(cause), || Point::Send {
                actor: "Worker",
                message: "Refresh",
            });
            (
                guinea_trace::thread_id(),
                format!("{:?}", std::thread::current().id()),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    let declaration = guinea_trace::CodeLocation {
        file: file!(),
        line: line!(),
        column: column!(),
        crate_dir: Some(env!("CARGO_MANIFEST_DIR")),
    };
    let child = guinea_trace::enter_under_at(Some(cause), declaration, || Point::Handle {
        actor: "Worker",
        message: "Refresh",
    });
    // Even crossing closes must not confuse Perfetto's per-track stack.
    drop(action);
    drop(child);
    recording
        .counter("memory.private_bytes", guinea_trace::now(), 4096.0)
        .unwrap();
    let bytes = recording.finish().unwrap();
    let trace = ProtoTrace::parse_from_bytes(&bytes).unwrap();
    let worker_thread = trace
        .packet
        .iter()
        .filter(|packet| packet.has_track_descriptor())
        .map(|packet| packet.track_descriptor())
        .filter(|descriptor| descriptor.thread.is_some())
        .map(|descriptor| descriptor.thread.as_ref().unwrap())
        .find(|thread| thread.tid() == worker.0 as i32)
        .unwrap();
    let id_kind = if cfg!(windows) { "OS" } else { "Trace" };
    assert_eq!(
        worker_thread.thread_name(),
        format!("scan-worker · {} · {id_kind} {}", worker.1, worker.0)
    );
    let events: Vec<_> = trace
        .packet
        .iter()
        .filter(|packet| packet.has_track_event())
        .map(|packet| packet.track_event())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.type_() == Type::TYPE_SLICE_BEGIN)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.type_() == Type::TYPE_SLICE_END)
            .count(),
        2
    );
    let begin_tracks: Vec<_> = events
        .iter()
        .filter(|e| e.type_() == Type::TYPE_SLICE_BEGIN)
        .map(|e| e.track_uuid())
        .collect();
    assert_ne!(begin_tracks[0], begin_tracks[1]);
    let begins: Vec<_> = events
        .iter()
        .filter(|e| e.type_() == Type::TYPE_SLICE_BEGIN)
        .collect();
    assert_eq!(
        begins.iter().map(|e| e.name()).collect::<Vec<_>>(),
        ["action · Refresh", "Refresh → Worker"]
    );
    assert_eq!(
        begins[1]
            .debug_annotations
            .iter()
            .find(|arg| arg.name() == "guinea_kind")
            .unwrap()
            .string_value(),
        "handle"
    );
    let path = begins[1]
        .debug_annotations
        .iter()
        .find(|arg| arg.name() == "source_path")
        .unwrap();
    assert_eq!(
        std::path::Path::new(path.string_value()),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perfetto.rs")
    );
    let line = begins[1]
        .debug_annotations
        .iter()
        .find(|arg| arg.name() == "line")
        .unwrap();
    assert_eq!(line.uint_value(), declaration.line as u64);
    let outgoing: Vec<_> = events
        .iter()
        .flat_map(|e| e.flow_ids.iter().copied())
        .collect();
    let incoming: Vec<_> = events
        .iter()
        .flat_map(|e| e.terminating_flow_ids.iter().copied())
        .collect();
    assert_eq!(outgoing, incoming);
    assert_eq!(outgoing.len(), 2);
    let counter = events
        .iter()
        .find(|e| e.type_() == Type::TYPE_COUNTER)
        .unwrap();
    assert_eq!(counter.double_counter_value(), 4096.0);
    assert!(
        sink.take_elsewhere().records.is_empty(),
        "streaming alone does not fill the UI queue"
    );
    assert!(!sink.is_observed());
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

#[test]
fn timeline_names_identify_actions_events_and_streams() {
    let sink = Sink::new();
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    let _current = record_into(&sink);
    for point in [
        Point::Action { message: "Add" },
        Point::Publish {
            event: "Changed",
            subscribers: 2,
        },
        Point::Deliver { event: "Changed" },
        Point::Pull {
            source: 7,
            to: "Worker",
            output: "Tick",
        },
        Point::Push { reducer: "Board" },
        Point::Spawn {
            actor: "Worker",
            actor_id: 1,
            output: "Saved",
        },
    ] {
        mark(|| point);
    }
    let trace = ProtoTrace::parse_from_bytes(&recording.finish().unwrap()).unwrap();
    let names: Vec<_> = trace
        .packet
        .iter()
        .filter(|packet| packet.has_track_event())
        .map(|packet| packet.track_event().name())
        .collect();
    assert_eq!(
        names,
        [
            "action · Add",
            "publish · Changed",
            "deliver · Changed",
            "pull · Tick → Worker · source #7",
            "push · Board",
            "spawn · Saved → Worker"
        ]
    );
    let pull = trace
        .packet
        .iter()
        .filter(|packet| packet.has_track_event())
        .map(|packet| packet.track_event())
        .find(|event| {
            event
                .debug_annotations
                .iter()
                .any(|arg| arg.name() == "guinea_kind" && arg.string_value() == "pull")
        })
        .unwrap();
    let source = pull
        .debug_annotations
        .iter()
        .find(|arg| arg.name() == "source")
        .unwrap();
    assert_eq!(source.uint_value(), 7);
}

impl Write for Shared {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn packets_are_written_before_finish_and_unfinished_work_stays_open() {
    let sink = Sink::new();
    let output = Shared::default();
    let recording = Recording::start(&sink, output.clone()).unwrap();
    let _current = record_into(&sink);
    let action = enter(|| Point::Action {
        message: "Still running",
    });
    let snapshot = output.0.lock().unwrap().clone();
    assert!(
        ProtoTrace::parse_from_bytes(&snapshot)
            .unwrap()
            .packet
            .iter()
            .any(|p| p.has_track_event())
    );
    recording.finish().unwrap();
    let before = output.0.lock().unwrap().len();
    drop(action);
    assert_eq!(output.0.lock().unwrap().len(), before);
}

struct Fails {
    remaining: usize,
}
impl Write for Fails {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::other("disk full"));
        }
        let count = bytes.len().min(self.remaining);
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn observer_write_errors_reach_finish_without_panicking_in_application_code() {
    let sink = Sink::new();
    let recording = Recording::start(&sink, Fails { remaining: 128 }).unwrap();
    let _current = record_into(&sink);
    for _ in 0..10 {
        mark(|| Point::Action { message: "Refresh" });
    }
    assert_eq!(
        recording.finish().err().unwrap().kind(),
        io::ErrorKind::Other
    );
}

#[test]
fn streaming_coexists_with_ui_observers_and_does_not_steal_their_queue() {
    let sink = Sink::new();
    let _observer = sink.observe(|_| {});
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    let other = sink.clone();
    std::thread::spawn(move || {
        let _current = record_into(&other);
        mark(|| Point::Push { reducer: "Count" });
    })
    .join()
    .unwrap();
    let queue = sink.take_elsewhere();
    assert_eq!(queue.records.len(), 1);
    assert!(matches!(queue.records[0].1, Trace::Mark(_)));
    assert!(!recording.finish().unwrap().is_empty());
}

/// Runs against the actual Perfetto binary, not our protobuf decoder.
/// Set TRACE_PROCESSOR to the v59+ executable, then run this ignored test.
#[test]
#[ignore = "requires official Trace Processor v59+ (TRACE_PROCESSOR)"]
fn trace_processor_imports_intervals_flows_args_and_counters_without_errors() {
    let processor =
        std::env::var_os("TRACE_PROCESSOR").expect("set TRACE_PROCESSOR to the executable");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.pftrace");
    let sink = Sink::new();
    let recording = Recording::start(&sink, std::fs::File::create(&path).unwrap()).unwrap();
    let _current = record_into(&sink);
    let action = enter(|| Point::Action { message: "Refresh" });
    let parent = action.id();
    let background = sink.clone();
    std::thread::Builder::new()
        .name("profile-worker".into())
        .spawn(move || {
            let _current = record_into(&background);
            let _handle = guinea_trace::enter_under(Some(parent), || Point::Handle {
                actor: "Worker",
                message: "Refresh",
            });
            mark(|| Point::Push { reducer: "Count" });
        })
        .unwrap()
        .join()
        .unwrap();
    drop(action);
    recording
        .counter("memory.private_bytes", guinea_trace::now(), 4096.0)
        .unwrap();
    recording.finish().unwrap();
    let query = dir.path().join("validate.sql");
    std::fs::write(&query, include_str!("perfetto/validate.sql")).unwrap();
    let output = std::process::Command::new(processor)
        .args(["query", "--format", "csv"])
        .arg("-f")
        .arg(query)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .replace('\r', ""),
        "\"intervals\",\"marks\",\"flows\",\"counters\",\"actors\",\"durations\",\"threads\",\"healthy\"\n1,1,1,1,1,1,1,1"
    );
}
