#![cfg(feature = "perfetto")]
use guinea_trace::perfetto::{Recording, merge_cpu, merge_cpu_raw};
use guinea_trace::{Point, Sink, enter, record_into};
use protobuf::Message;
use serde_json::json;

#[test]
#[cfg(any(windows, target_os = "linux"))]
#[ignore = "requires official Trace Processor v59+ (TRACE_PROCESSOR)"]
fn dashboard_ranks_nearest_actions_once_and_excludes_off_cpu_waits() {
    let sink = Sink::new();
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    let _sink = record_into(&sink);
    let outer = enter(|| Point::Action { message: "Refresh" });
    let first = guinea_trace::now().as_nanos() as f64 / 1e6;
    let inner = enter(|| Point::Action { message: "Sort" });
    let second = guinea_trace::now().as_nanos() as f64 / 1e6;
    drop(inner);
    let waiting = guinea_trace::now().as_nanos() as f64 / 1e6;
    drop(outer);
    let outside = guinea_trace::now().as_nanos() as f64 / 1e6;
    let start = guinea_trace::started_at()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0;
    let profile = json!({"meta":{"startTime":start,"interval":2.0},"threads":[{
        "pid":std::process::id(),"tid":guinea_trace::thread_id(),
        "samples":{"time":[first,second,waiting,outside],"stack":[2,2,2,2],"threadCPUDelta":[2000,2000,0,2000]},
        "stackTable":{"frame":[0,1,2],"prefix":[null,0,1]},"frameTable":{"func":[0,1,2],"address":[1,2,3]},
        "funcTable":{"name":[0,1,2],"resource":[-1,-1,-1]},"stringArray":["app::main","guinea_core::feature::renamed_entry","work"]
    }]});
    let merged = merge_cpu(&profile, None, &recording.finish().unwrap()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("combined.pftrace");
    std::fs::write(&path, merged).unwrap();
    let query = format!(
        "CREATE PERFETTO TABLE hot AS {};\nSELECT action, samples, estimated_cpu_ms, ROUND(process_cpu_percent, 2) AS process_cpu_percent, invocations, estimated_cpu_ms_per_invocation FROM hot ORDER BY action;\nSELECT COUNT(*) AS cpu_samples FROM perf_sample;\nSELECT name, value FROM stats WHERE severity = 'error' AND value != 0;",
        include_str!("../sql/top_cpu_actions.sql")
            .trim()
            .trim_end_matches(';')
    );
    let output =
        std::process::Command::new(std::env::var_os("TRACE_PROCESSOR").expect("TRACE_PROCESSOR"))
            .args(["query", "--format", "csv"])
            .arg(&path)
            .arg(query)
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    insta::assert_snapshot!(
        "hot_cpu_actions",
        String::from_utf8(output.stdout).unwrap().replace('\r', "")
    );
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn cpu_attribution_follows_execution_and_leaves_waiting_and_other_threads_alone() {
    let sink = Sink::new();
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    let _sink = record_into(&sink);
    let handler = enter(|| Point::Handle {
        actor: "ProcessesActor",
        message: "RemoteScanResult",
    });
    let cause = handler.id();
    let inside = guinea_trace::now().as_nanos() as f64 / 1e6;
    drop(handler);
    let waiting = guinea_trace::now().as_nanos() as f64 / 1e6;
    let resumed = guinea_trace::resume(Some(cause));
    let polled = guinea_trace::now().as_nanos() as f64 / 1e6;
    drop(resumed);
    let outside = guinea_trace::now().as_nanos() as f64 / 1e6;
    let bytes = recording.finish().unwrap();
    let start = guinea_trace::started_at()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0;
    let thread = json!({"pid":std::process::id(),"tid":guinea_trace::thread_id(),
        "samples":{"time":[inside,waiting,polled,outside],"stack":[2,2,2,2]},
        "stackTable":{"frame":[0,1,2],"prefix":[null,0,1]},"frameTable":{"func":[0,1,2],"address":[1,2,3]},
        "funcTable":{"name":[0,1,2],"resource":[-1,-1,-1]},
        "stringArray":["app::main","guinea_core::actor::renamed_entry","environments_from_scan"]});
    let mut other = thread.clone();
    other["tid"] = json!(u32::MAX);
    let profile = json!({"meta":{"startTime":start,"interval":2.0},"threads":[thread,other]});
    let merged = merge_cpu(&profile, None, &bytes).unwrap();
    let trace = perfetto_protos::trace::Trace::parse_from_bytes(&merged).unwrap();
    let intern = trace
        .packet
        .iter()
        .find_map(|packet| packet.interned_data.as_ref())
        .unwrap();
    let samples: Vec<_> = trace
        .packet
        .iter()
        .filter(|packet| packet.has_perf_sample())
        .map(|packet| packet.perf_sample())
        .collect();
    assert_eq!(samples.len(), 8);
    let stacks: Vec<Vec<String>> = samples
        .iter()
        .map(|sample| {
            let stack = intern
                .callstacks
                .iter()
                .find(|stack| stack.iid() == sample.callstack_iid())
                .unwrap()
                .frame_ids
                .iter()
                .map(|id| {
                    let frame = intern
                        .frames
                        .iter()
                        .find(|frame| frame.iid() == *id)
                        .unwrap();
                    let name = intern
                        .function_names
                        .iter()
                        .find(|name| name.iid() == frame.function_name_id())
                        .unwrap();
                    String::from_utf8(name.str().to_vec()).unwrap()
                })
                .collect();
            stack
        })
        .collect();
    insta::assert_debug_snapshot!("execution_stack_projection", stacks);
    let raw = perfetto_protos::trace::Trace::parse_from_bytes(
        &merge_cpu_raw(&profile, None, &bytes).unwrap(),
    )
    .unwrap();
    let raw_intern = raw
        .packet
        .iter()
        .find_map(|packet| packet.interned_data.as_ref())
        .unwrap();
    let raw_names: Vec<_> = raw_intern
        .function_names
        .iter()
        .map(|name| String::from_utf8(name.str().to_vec()).unwrap())
        .collect();
    assert_eq!(
        raw_names,
        [
            "app::main",
            "guinea_core::actor::renamed_entry",
            "environments_from_scan"
        ]
    );
    assert_eq!(
        raw_intern
            .frames
            .iter()
            .map(|frame| frame.rel_pc())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        raw.packet
            .iter()
            .filter(|packet| packet.has_perf_sample())
            .count(),
        samples.len()
    );
    assert_eq!(
        trace.packet.iter().filter(|p| p.has_track_event()).count(),
        perfetto_protos::trace::Trace::parse_from_bytes(&bytes)
            .unwrap()
            .packet
            .iter()
            .filter(|p| p.has_track_event())
            .count()
    );
}
