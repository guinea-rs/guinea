use std::{io, time::Duration};

use guinea::app::Harness;
use guinea::observability::trace::{
    self,
    allocations::{Allocator, Config, Profiler, allocator},
    perfetto::Recording,
};
use todo_core::profiling::{session, verify_saved};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

type Trace = Recording<io::BufWriter<std::fs::File>>;

fn sample(recording: &Trace, profiler: &Profiler) -> io::Result<()> {
    recording.memory(profiler)?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::GetCurrentProcess,
        };
        let mut memory: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        memory.cb = std::mem::size_of_val(&memory) as u32;
        if unsafe {
            K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut memory).cast(), memory.cb)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let at = trace::now();
        recording.counter(
            "Process / private commit bytes",
            at,
            memory.PrivateUsage as f64,
        )?;
        recording.counter(
            "Process / working set bytes",
            at,
            memory.WorkingSetSize as f64,
        )?;
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut tasks = 1500;
    let mut detailed = false;
    for arg in std::env::args().skip(1) {
        if arg == "--state-details" {
            detailed = true;
        } else {
            tasks = arg.parse()?;
        }
    }
    let storage = tempfile::tempdir()?;
    let path = storage.path().join("todo.json");
    let profiler = Profiler::install(Config {
        history_capacity: 0,
    })?;
    let mut app = Harness::new(42);
    let sink = trace::sink_of(app.application_scope()).expect("application scope has a trace sink");
    let recording = trace::perfetto::from_env(&sink)?
        .ok_or_else(|| anyhow::anyhow!("run with cargo xtask profile"))?;
    let states = app.record_states(guinea::observability::state::Config::default());
    if detailed {
        states.set_mode(guinea::observability::state::Mode::Detailed);
    }
    sample(&recording, &profiler)?;
    let report = session(&mut app, &path, tasks, |phase| {
        sample(&recording, &profiler)?;
        eprintln!("{phase}");
        Ok(())
    })?;
    // Advance only application deadlines, never invent timing spans for the trace.
    app.advance(Duration::from_millis(300));
    drop(app);
    sample(&recording, &profiler)?;
    verify_saved(&path, &report)?;
    drop(states);
    sample(&recording, &profiler)?;
    recording.finish()?;
    println!(
        "{} tasks, revision {}, {} search matches; persisted state verified",
        report.tasks, report.revision, report.matches
    );
    Ok(())
}
