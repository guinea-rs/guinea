//! Alone in its binary: an observer anywhere in the process would make every
//! point recorded regardless of the filter.

use std::sync::{Arc, Mutex};

use guinea_trace::{Point, mark};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Default)]
struct Targets(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Targets {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        self.0.lock().unwrap().push(event.metadata().target().to_string());
    }
}

#[test]
fn a_filter_that_lets_one_kind_through_gets_that_kind() {
    let written = Targets::default();
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("guinea::send=debug"))
        .with(written.clone());

    tracing::subscriber::with_default(subscriber, || {
        mark(|| Point::Send {
            actor: "ProcessActor",
            message: "Kill",
        });
    });

    assert_eq!(
        *written.0.lock().unwrap(),
        ["guinea::send"],
        "RUST_LOG=guinea::send=debug wrote no send"
    );
}

#[test]
fn the_whole_of_guinea_at_debug_gets_it_for_comparison() {
    let written = Targets::default();
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("guinea=debug"))
        .with(written.clone());

    tracing::subscriber::with_default(subscriber, || {
        mark(|| Point::Send {
            actor: "ProcessActor",
            message: "Kill",
        });
    });

    assert_eq!(*written.0.lock().unwrap(), ["guinea::send"]);
}
