//! One JSON object per line.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::layer::Context;

use crate::sink::is_point_target;

const RESERVED: [&str; 4] = ["ts", "level", "target", "cause"];

/// A layer writing every event as a JSON object on its own line: `ts`,
/// `level`, `target`, then `cause` - the point an application's own event
/// happened under - then the event's fields with their own types.
///
/// ```
/// use tracing_subscriber::prelude::*;
/// # let dir = tempfile::tempdir()?;
/// # let path = dir.path().join("app.log");
///
/// let log = std::fs::File::create(&path)?;
/// tracing_subscriber::registry()
///     .with(guinea_trace::json(log))
///     .init();
///
/// tracing::info!(pid = 42, "process killed");
/// # let line: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
/// # assert_eq!(line["level"], "INFO");
/// # assert_eq!(line["message"], "process killed");
/// # assert_eq!(line["pid"], 42);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// A line that cannot be written is said on stderr, once for each run of
/// lines that fail, since tracing is what would have carried the news.
pub fn json<W>(writer: W) -> Json<W>
where
    W: for<'w> MakeWriter<'w> + 'static,
{
    Json {
        writer,
        failing: AtomicBool::new(false),
    }
}

pub struct Json<W> {
    writer: W,
    failing: AtomicBool,
}

impl<W> Json<W> {
    fn wrote(&self, written: io::Result<()>) {
        match written {
            Ok(()) => self.failing.store(false, Ordering::Relaxed),
            Err(error) if !self.failing.swap(true, Ordering::Relaxed) => {
                eprintln!(
                    "guinea-trace: a JSON line could not be written ({error}); \
                     the lines failing after it go unsaid until one is written"
                );
            }
            Err(_) => {}
        }
    }
}

impl<S, W> tracing_subscriber::Layer<S> for Json<W>
where
    S: tracing::Subscriber,
    W: for<'w> MakeWriter<'w> + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let meta = event.metadata();

        let mut ts = String::new();
        if SystemTime.format_time(&mut Writer::new(&mut ts)).is_err() {
            ts.clear();
        }

        let mut line = Line(vec![
            ("ts", Value::from(ts)),
            ("level", Value::from(meta.level().as_str())),
            ("target", Value::from(meta.target())),
        ]);
        if !is_point_target(meta.target())
            && let Some(cause) = crate::current()
        {
            line.0.push(("cause", Value::from(cause.get())));
        }

        event.record(&mut line);

        let written = line
            .into_bytes()
            .map_err(io::Error::from)
            .and_then(|bytes| self.writer.make_writer_for(meta).write_all(&bytes));
        self.wrote(written);
    }
}

struct Line(Vec<(&'static str, Value)>);

impl Line {
    fn put(&mut self, field: &Field, value: Value) {
        let name = field.name();
        if RESERVED.contains(&name) || self.0.iter().any(|(taken, _)| *taken == name) {
            return;
        }

        self.0.push((name, value));
    }

    /// In the order the fields came, which a map would not keep.
    fn into_bytes(self) -> serde_json::Result<Vec<u8>> {
        let mut bytes = vec![b'{'];
        for (at, (name, value)) in self.0.iter().enumerate() {
            if at > 0 {
                bytes.push(b',');
            }
            serde_json::to_writer(&mut bytes, name)?;
            bytes.push(b':');
            serde_json::to_writer(&mut bytes, value)?;
        }
        bytes.extend_from_slice(b"}\n");

        Ok(bytes)
    }
}

impl Visit for Line {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, Value::from(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::from(value));
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        let value = i64::try_from(value).map_or_else(|_| Value::from(value.to_string()), Value::from);
        self.put(field, value);
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        let value = u64::try_from(value).map_or_else(|_| Value::from(value.to_string()), Value::from);
        self.put(field, value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, Value::from(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, Value::from(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, Value::from(format!("{value:?}")));
    }
}
