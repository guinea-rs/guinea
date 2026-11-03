//! CPU workload for the external sampler, with cross-thread Guinea causality.
use guinea_trace::{Point, Sink, enter, mark, record_into};
use std::time::{Duration, Instant};

#[inline(never)]
fn environments_from_scan(milliseconds: u64) {
    let until = Instant::now() + Duration::from_millis(milliseconds);
    let mut checksum = 1_u64;
    while Instant::now() < until {
        for value in 0..4096 {
            checksum = std::hint::black_box(
                checksum
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(value),
            );
        }
    }
    std::hint::black_box(checksum);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let recording =
        guinea_trace::perfetto::from_env(&sink)?.ok_or("run through cargo xtask profile")?;
    let _sink = record_into(&sink);
    for _ in 0..4 {
        let action = enter(|| Point::Action {
            message: "RefreshProcesses",
        });
        let carried = guinea_trace::carry();
        std::thread::spawn(move || {
            let _context = carried.resume();
            let _work = enter(|| Point::Span {
                name: "scan_linux",
                level: tracing::Level::DEBUG,
                target: "demo::processes",
                file: Some(file!()),
                line: Some(line!()),
                module: Some(module_path!()),
                fields: String::new(),
            });
            environments_from_scan(150);
        })
        .join()
        .unwrap();
        let send = mark(|| Point::Send {
            actor: "ProcessesActor",
            message: "RemoteScanResult",
        });
        {
            let _handle = guinea_trace::enter_under(Some(send), || Point::Handle {
                actor: "ProcessesActor",
                message: "RemoteScanResult",
            });
            environments_from_scan(100);
            mark(|| Point::Push {
                reducer: "ProcessesState",
            });
        }
        drop(action);
        environments_from_scan(25);
        {
            let _action = enter(|| Point::Action {
                message: "SortProcesses",
            });
            let _handler = enter(|| Point::Handle {
                actor: "ProcessesActor",
                message: "Sort",
            });
            environments_from_scan(60);
        }
    }
    drop(_sink);
    recording.finish()?;
    Ok(())
}
