//! cargo run -p guinea-trace --features perfetto --example perfetto -- output.pftrace
#[cfg(feature = "perfetto")]
fn main() -> std::io::Result<()> {
    use guinea_trace::perfetto::Recording;
    use guinea_trace::{Point, Sink, enter, mark, mark_under, record_into};
    let sink = Sink::new();
    let recording = match guinea_trace::perfetto::from_env(&sink)? {
        Some(recording) => recording,
        None => {
            let path = std::env::args_os()
                .nth(1)
                .expect("pass an output .pftrace path");
            Recording::start(&sink, std::io::BufWriter::new(std::fs::File::create(path)?))?
        }
    };
    let _current = record_into(&sink);
    for value in 0..3 {
        let action = enter(|| Point::Action { message: "Refresh" });
        let send = mark(|| Point::Send {
            actor: "Worker",
            message: "Refresh",
        });
        let background = sink.clone();
        std::thread::spawn(move || {
            let _current = record_into(&background);
            let _handle = guinea_trace::enter_under(Some(send), || Point::Handle {
                actor: "Worker",
                message: "Refresh",
            });
            mark(|| Point::Push { reducer: "Count" });
            std::thread::sleep(std::time::Duration::from_millis(1));
        })
        .join()
        .unwrap();
        mark_under(Some(action.id()), || Point::Plugin {
            kind: "scenario.step",
            fields: vec![("iteration", value.to_string())],
        });
        recording.counter("scenario.iteration", guinea_trace::now(), value as f64)?;
    }
    recording.finish()?;
    Ok(())
}

#[cfg(not(feature = "perfetto"))]
fn main() {
    eprintln!("enable --features perfetto");
}
