#![cfg(all(feature = "allocations", feature = "perfetto"))]

use guinea_trace::allocations::{Allocator, Config, Group, Profiler, allocator, of_cause};
use guinea_trace::perfetto::Recording;
use guinea_trace::{Point, Sink, enter, record_into};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

#[test]
#[ignore = "requires official Trace Processor v59+ (TRACE_PROCESSOR)"]
fn memory_is_queryable_on_the_original_interval_without_per_scope_tracks() {
    let processor = std::env::var_os("TRACE_PROCESSOR").expect("set TRACE_PROCESSOR");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.pftrace");
    let profiler = Profiler::install(Config::default()).unwrap();
    let sink = Sink::new();
    let group = Group::new(42, None, sink.clone()).unwrap();
    let recording = Recording::start(&sink, std::fs::File::create(&path).unwrap()).unwrap();
    let _current = record_into(&sink);
    let action = enter(|| Point::Action {
        message: "Allocate",
    });
    let id = action.id();
    let scoped = group.run(|| Box::new([0_u8; 1024]));
    let unscoped = Box::new([0_u8; 512]);
    std::hint::black_box((&scoped, &unscoped));
    recording.memory(&profiler).unwrap();
    drop(action);
    std::thread::spawn(move || {
        drop(scoped);
        drop(unscoped);
    })
    .join()
    .unwrap();
    recording.memory(&profiler).unwrap();
    let stats = of_cause(id).unwrap();
    assert_eq!(stats.allocated_bytes, 1536);
    assert_eq!(stats.freed_bytes, 1536);
    assert_eq!(stats.live_bytes, 0);
    assert_eq!(profiler.snapshot()[0].allocated_bytes, 1024);
    assert_eq!(profiler.snapshot()[0].live_bytes, 0);
    recording.finish().unwrap();
    let query = dir.path().join("memory.sql");
    std::fs::write(&query, include_str!("perfetto/memory.sql")).unwrap();
    let output = std::process::Command::new(processor)
        .args(["query", "--format", "csv", "-f"])
        .arg(query)
        .arg(path)
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
        "\"valid\"\n1"
    );
}
