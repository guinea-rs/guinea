#![cfg(all(feature = "allocations", feature = "perfetto"))]

use guinea_trace::Sink;
use guinea_trace::allocations::{Allocator, Config, Declaration, Group, Profiler, allocator};
use guinea_trace::perfetto::Recording;

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

#[test]
#[ignore = "requires official Trace Processor v59+ (TRACE_PROCESSOR)"]
fn metrics_distinguish_growth_churn_closed_retention_and_incomplete_samples() {
    let processor = std::env::var_os("TRACE_PROCESSOR").expect("set TRACE_PROCESSOR");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("growth.pftrace");
    let profiler = Profiler::install(Config::default()).unwrap();
    let sink = Sink::new();
    let recording = Recording::start(&sink, std::fs::File::create(&path).unwrap()).unwrap();
    let groups: Vec<_> = [
        ("Growth", "feature"),
        ("Churn", "plugin"),
        ("Held", "segment"),
        ("Cache", "segment"),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, (name, kind))| {
        let group = Group::new(id as u64, None, sink.clone()).unwrap();
        group.declare(Declaration {
            kind: "role",
            name: "Window root",
            file: None,
            line: None,
            column: None,
            crate_dir: None,
        });
        group.declare(Declaration {
            kind,
            name,
            file: Some(file!()),
            line: Some(line!()),
            column: Some(column!()),
            crate_dir: Some(env!("CARGO_MANIFEST_DIR")),
        });
        group
    })
    .collect();
    let held = groups[2].run(|| Box::new([0_u8; 128]));
    let cache = groups[3].run(|| Box::new([0_u8; 64]));
    recording.memory(&profiler).unwrap();
    let first = groups[0].run(|| Box::new([0_u8; 512]));
    let temporary = groups[1].run(|| Box::new([0_u8; 4096]));
    std::hint::black_box(&temporary);
    drop(temporary);
    groups[2].close();
    recording.memory(&profiler).unwrap();
    let second = groups[0].run(|| Box::new([0_u8; 512]));
    let temporary = groups[1].run(|| Box::new([0_u8; 4096]));
    std::hint::black_box(&temporary);
    drop(temporary);
    recording.memory(&profiler).unwrap();
    recording
        .counter("test.window.end", guinea_trace::now(), 1.0)
        .unwrap();
    drop((first, second, held, cache));
    groups[0].close();
    groups[3].close();
    let late = Group::new(99, None, sink).unwrap();
    late.declare(Declaration {
        kind: "role",
        name: "Application",
        file: None,
        line: None,
        column: None,
        crate_dir: None,
    });
    let late_data = late.run(|| Box::new([0_u8; 2048]));
    recording.memory(&profiler).unwrap();
    recording.finish().unwrap();
    drop(late_data);
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/guinea");
    let query = dir.path().join("validate.sql");
    std::fs::write(&query, include_str!("perfetto/growth.sql")).unwrap();
    let output = std::process::Command::new(&processor)
        .args(["query", "--format", "csv", "--add-sql-package"])
        .arg(&package)
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
        "\"valid\"\n1"
    );
    let output = std::process::Command::new(processor)
        .args(["summarize", "--metrics-v2", "all", "--add-sql-package"])
        .arg(package)
        .arg(path)
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/memory_metrics.pbtxt"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty());
}
