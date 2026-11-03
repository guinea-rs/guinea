//! cargo run -p guinea-trace --features perfetto,allocations --example perfetto_allocations -- output.pftrace
//! Add --retain-page after the path to demonstrate a closed scope retaining
//! memory at capture end. It is released after capture, not leaked by the demo.
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::time::Duration;

use guinea_trace::allocations::{
    Allocator, Config, Declaration, Group, Profiler, Totals, allocator,
};
use guinea_trace::perfetto::Recording;
use guinea_trace::{Point, Sink, enter, record_into};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);
const MIB: usize = 1024 * 1024;
type Trace = Recording<BufWriter<std::fs::File>>;

fn sample(profiler: &Profiler, recording: &Trace) -> io::Result<()> {
    recording.memory(profiler)?;
    system_memory(recording)
}

#[cfg(windows)]
fn system_memory(recording: &Trace) -> io::Result<()> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of_val(&counters) as u32;
    if unsafe {
        K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), counters.cb)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let at = guinea_trace::now();
    recording.counter(
        "Process / private commit bytes",
        at,
        counters.PrivateUsage as f64,
    )?;
    recording.counter(
        "Process / working set bytes",
        at,
        counters.WorkingSetSize as f64,
    )
}

#[cfg(not(windows))]
fn system_memory(_: &Trace) -> io::Result<()> {
    Ok(())
}

fn hold(profiler: &Profiler, recording: &Trace) -> io::Result<()> {
    // Periodic samples as well as explicit phase boundaries.
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(25));
        sample(profiler, recording)?;
    }
    Ok(())
}

fn block(group: &Group, size: usize) -> Vec<u8> {
    group.run(|| std::hint::black_box(vec![0x5a; size]))
}

fn totals(stats: &Totals) -> serde_json::Value {
    serde_json::json!({
        "allocations": stats.allocations, "deallocations": stats.deallocations,
        "allocated_bytes": stats.allocated_bytes, "freed_bytes": stats.freed_bytes,
        "live_bytes": stats.live_bytes, "peak_bytes": stats.peak_bytes,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().expect("pass an output .pftrace path"));
    let retain_page = match args.next() {
        None => false,
        Some(flag) if flag == "--retain-page" => true,
        _ => return Err("expected --retain-page or no extra argument".into()),
    };
    // Normal capture: no allocation-per-marker noise. Cause/scope counters remain
    // exact even with the raw allocation history disabled.
    let profiler = Profiler::install(Config::default())?;
    let sink = Sink::new();
    let recording = Recording::start(&sink, BufWriter::new(std::fs::File::create(&path)?))?;
    let _current = record_into(&sink);
    let cache = Group::new(1, None, sink.clone()).unwrap();
    let page = Group::new(2, Some(&cache), sink.clone()).unwrap();
    // This backend-independent demo creates groups directly. Real mounted
    // segments/features supply their existing lexical metadata automatically.
    for (group, name) in [(&cache, "cache"), (&page, "page")] {
        group.declare(Declaration {
            kind: "scope",
            name,
            file: None,
            line: None,
            column: None,
            crate_dir: None,
        });
    }
    sample(&profiler, &recording)?;
    let retained;
    let mut page_data;
    {
        let _phase = enter(|| Point::Action {
            message: "Load cache and page",
        });
        retained = block(&cache, 2 * MIB);
        page_data = Some(block(&page, MIB));
        sample(&profiler, &recording)?;
        hold(&profiler, &recording)?;
    }
    {
        let _phase = enter(|| Point::Action {
            message: "Temporary page allocation",
        });
        let temporary = block(&page, 3 * MIB);
        sample(&profiler, &recording)?;
        hold(&profiler, &recording)?;
        drop(temporary);
        sample(&profiler, &recording)?;
        hold(&profiler, &recording)?;
    }
    {
        let _phase = enter(|| Point::Action {
            message: "Close page; keep cache",
        });
        page.close();
        if !retain_page {
            drop(page_data.take());
        }
        sample(&profiler, &recording)?;
        hold(&profiler, &recording)?;
    }
    {
        let _phase = enter(|| Point::Action {
            message: "Release cache",
        });
        cache.close();
        drop(retained);
        sample(&profiler, &recording)?;
        hold(&profiler, &recording)?;
    }
    let scopes = profiler.snapshot();
    assert_eq!(
        scopes.iter().map(|stats| stats.live_bytes).sum::<u64>(),
        if retain_page { MIB as u64 } else { 0 }
    );
    assert_eq!(
        scopes
            .iter()
            .map(|stats| stats.allocated_bytes)
            .sum::<u64>(),
        (6 * MIB) as u64
    );
    let summary = serde_json::json!({
        "coverage": "Requested Rust bytes; native heaps excluded. Tracing attribution excludes recursive profiler/backend bookkeeping, which is included in Rust heap totals.",
        "heap": totals(&guinea_trace::allocations::heap()),
        "tracing": totals(&profiler.tracing()),
        "scopes": scopes.iter().map(|stats| serde_json::json!({
            "group": stats.id, "scope": stats.scope, "parent_group": stats.parent,
            "closed": stats.closed, "allocated_bytes": stats.allocated_bytes,
            "freed_bytes": stats.freed_bytes, "live_bytes": stats.live_bytes,
            "peak_bytes": stats.peak_bytes,
            "declarations": stats.declarations.iter().map(|site| serde_json::json!({
                "kind": site.kind, "name": site.name, "file": site.file,
                "line": site.line, "column": site.column, "crate_dir": site.crate_dir,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "causes": profiler.causes().iter().map(|(id, stats)| serde_json::json!({
            "cause": id.get(), "memory": totals(stats),
        })).collect::<Vec<_>>(),
    });
    // Scope/cause tables belong to introspection, not one graph per scope.
    std::fs::write(
        path.with_extension("memory.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    recording.finish()?;
    drop(page_data);
    Ok(())
}
