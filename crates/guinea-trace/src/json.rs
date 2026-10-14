//! One JSON object per line.

use std::io::Write;

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
/// ```no_run
/// use tracing_subscriber::prelude::*;
///
/// let log = std::fs::File::create("app.log").unwrap();
/// tracing_subscriber::registry()
///     .with(guinea_trace::json(log))
///     .init();
/// ```
pub fn json<W>(writer: W) -> Json<W>
where
    W: for<'w> MakeWriter<'w> + 'static,
{
    Json { writer }
}

pub struct Json<W> {
    writer: W,
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

        let Ok(bytes) = line.into_bytes() else {
            return;
        };

        let _ = self.writer.make_writer_for(meta).write_all(&bytes);
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json as value;
    use tracing_subscriber::layer::SubscriberExt;

    use crate::{Point, mark, resume};

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'w self) -> Buffer {
            self.clone()
        }
    }

    fn lines(buffer: &Buffer) -> Vec<(String, serde_json::Value)> {
        let bytes = buffer.0.lock().unwrap();
        String::from_utf8(bytes.clone())
            .unwrap()
            .lines()
            .map(|line| (line.to_string(), serde_json::from_str(line).unwrap()))
            .collect()
    }

    #[test]
    fn an_event_is_a_line_of_json_with_its_fields_typed_and_its_cause() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry().with(super::json(buffer.clone()));

        tracing::subscriber::with_default(subscriber, || {
            let action = mark(|| Point::Action { message: "Kill" });
            let _resumed = resume(Some(action));
            tracing::info!(target: "app", pid = 42, alive = false, name = %"init", "process killed");
        });

        let lines = lines(&buffer);
        assert_eq!(lines.len(), 2, "{lines:?}");

        let (_, point) = &lines[0];
        assert_eq!(point["target"], "guinea::action");
        assert_eq!(point["action"], "Kill");
        assert_eq!(point.get("cause"), None, "a point has its parent already");

        let (line, event) = &lines[1];
        let order: Vec<usize> = ["ts", "level", "target", "cause", "message", "pid", "alive", "name"]
            .iter()
            .map(|key| line.find(&format!("\"{key}\":")).expect(key))
            .collect();
        assert!(order.is_sorted(), "{line}");

        assert_eq!(event["level"], "INFO");
        assert_eq!(event["cause"], point["id"]);
        assert_eq!(event["message"], "process killed");
        assert_eq!(event["pid"], value!(42));
        assert_eq!(event["alive"], value!(false));
        assert_eq!(event["name"], "init");
    }

    fn stored() -> Point {
        Point::Plugin {
            kind: "store.set",
            fields: vec![("path", "settings/theme".into()), ("outside", "false".into())],
        }
    }

    #[test]
    fn a_plugin_point_is_a_line_with_its_kind_and_its_fields() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry().with(super::json(buffer.clone()));

        tracing::subscriber::with_default(subscriber, || {
            mark(stored);
        });

        let lines = lines(&buffer);
        let read = lines.first().map(|(_, point)| {
            (
                point["target"].as_str().map(str::to_owned),
                point["kind"].as_str().map(str::to_owned),
                point["fields"].as_str().map(str::to_owned),
            )
        });
        assert_eq!(
            read,
            Some((
                Some("guinea::plugin".into()),
                Some("store.set".into()),
                Some("path=settings/theme outside=false".into())
            )),
            "{lines:?}"
        );
    }

    #[test]
    fn a_plugin_point_reads_as_its_kind_and_its_fields() {
        assert_eq!(stored().to_string(), "store.set path=settings/theme outside=false");
        assert_eq!(stored().kind(), "store.set");
    }
}
