use std::sync::{Arc, Mutex};

use guinea_trace::{Point, mark, mark_under, resume};
use serde_json::json as value;
use tracing_subscriber::layer::SubscriberExt;

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
    let subscriber = tracing_subscriber::registry().with(guinea_trace::json(buffer.clone()));

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
    let subscriber = tracing_subscriber::registry().with(guinea_trace::json(buffer.clone()));

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

type Event = (String, Vec<(String, String)>);

#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<Event>>>);

struct Fields(Vec<(String, String)>);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{value:?}")));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Written {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut fields = Fields(Vec::new());
        event.record(&mut fields);

        let target = event.metadata().target().to_string();
        self.0.lock().unwrap().push((target, fields.0));
    }
}

#[test]
fn a_point_goes_to_tracing_as_its_kind_with_what_it_holds_as_fields() {
    let written = Written::default();
    let subscriber = tracing_subscriber::registry().with(written.clone());

    tracing::subscriber::with_default(subscriber, || {
        let send = mark(|| Point::Send {
            actor: "ProcessActor",
            message: "Kill",
        });
        mark_under(Some(send), || Point::Log {
            level: tracing::Level::INFO,
            target: "app",
            file: None,
            line: None,
            module: None,
            text: "already written by whoever logged it".into(),
        });
    });

    let written = written.0.lock().unwrap();
    let fields: Vec<(&str, &str)> = written[0]
        .1
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();

    assert_eq!(written.len(), 1, "a log point is not written back: {written:?}");
    assert_eq!(written[0].0, "guinea::send");
    assert!(fields.contains(&("actor", "ProcessActor")), "{fields:?}");
    assert!(fields.contains(&("msg", "Kill")), "{fields:?}");
    assert!(!fields.iter().any(|(name, _)| *name == "parent"), "no cause, no parent: {fields:?}");
    assert!(!fields.iter().any(|(name, _)| *name == "message"), "no prose: {fields:?}");
}
