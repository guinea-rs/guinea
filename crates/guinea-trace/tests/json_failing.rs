use std::io;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing_subscriber::layer::SubscriberExt;

const CHILD: &str = "GUINEA_TRACE_JSON_CHILD";

#[derive(Clone, Default)]
struct Disk(Arc<AtomicBool>);

impl io::Write for Disk {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.0.load(Ordering::Relaxed) {
            true => Ok(bytes.len()),
            false => Err(io::Error::other("the disk is full")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Disk {
    type Writer = Disk;

    fn make_writer(&'w self) -> Disk {
        self.clone()
    }
}

fn write_through_a_failing_disk() {
    let disk = Disk::default();
    let subscriber = tracing_subscriber::registry().with(guinea_trace::json(disk.clone()));

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "app", "first");
        tracing::info!(target: "app", "second");

        disk.0.store(true, Ordering::Relaxed);
        tracing::info!(target: "app", "written");

        disk.0.store(false, Ordering::Relaxed);
        tracing::info!(target: "app", "third");
        tracing::info!(target: "app", "fourth");
    });
}

#[test]
fn a_line_that_cannot_be_written_is_said_on_stderr_once_a_run_of_failures() {
    if std::env::var_os(CHILD).is_some() {
        write_through_a_failing_disk();
        return;
    }

    let ran = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_line_that_cannot_be_written_is_said_on_stderr_once_a_run_of_failures",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&ran.stderr);

    let said: Vec<&str> = stderr.lines().filter(|line| line.contains("the disk is full")).collect();
    assert_eq!(said.len(), 2, "one for each run of failures:\n{stderr}");
}
