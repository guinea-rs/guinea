//! What devtools read that is not a trace: panels a backend or a plugin
//! offers, for a root or for the whole application.
//!
//! What happened is in [`crate::trace`]; [`mark_anywhere`] is how something
//! off the UI thread adds to it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::trace::{self, Point};

pub use crate::trace::is_observed;

/// Something a backend knows about a window that the rest of guinea does not:
/// its component tree, say. Devtools draw it without interpreting it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    pub id: &'static str,
    pub title: &'static str,
    pub nodes: Vec<PanelNode>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelNode {
    pub label: String,
    pub kind: String,
    pub properties: Vec<(String, String)>,
    pub children: Vec<PanelNode>,
}

type Provider = Rc<dyn Fn() -> Option<Panel>>;

thread_local! {
    static PANELS: RefCell<Vec<(u64, Option<u64>, Provider)>> = const { RefCell::new(Vec::new()) };
    static NEXT_PANEL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn offer(root: Option<u64>, provider: impl Fn() -> Option<Panel> + 'static) -> PanelGuard {
    let id = NEXT_PANEL.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    PANELS.with(|panels| panels.borrow_mut().push((id, root, Rc::new(provider))));
    PanelGuard { id }
}

fn built(root: Option<u64>) -> Vec<Panel> {
    let providers: Vec<Provider> = PANELS.with(|panels| {
        panels
            .borrow()
            .iter()
            .filter(|(_, owner, _)| *owner == root)
            .map(|(_, _, provider)| provider.clone())
            .collect()
    });
    providers.iter().filter_map(|provider| provider()).collect()
}

/// Offers a panel for the root numbered `root`, for as long as the guard
/// lives. `provider` is asked only while devtools are reading.
#[must_use = "the panel is withdrawn when the guard is dropped"]
pub fn contribute(root: u64, provider: impl Fn() -> Option<Panel> + 'static) -> PanelGuard {
    offer(Some(root), provider)
}

/// Offers a panel about the whole application rather than one window: what a
/// plugin holds, say.
#[must_use = "the panel is withdrawn when the guard is dropped"]
pub fn contribute_to_app(provider: impl Fn() -> Option<Panel> + 'static) -> PanelGuard {
    offer(None, provider)
}

/// Every panel offered for `root`, built now.
pub fn panels(root: u64) -> Vec<Panel> {
    built(Some(root))
}

/// Every panel offered for the application, built now.
pub fn app_panels() -> Vec<Panel> {
    built(None)
}

pub struct PanelGuard {
    id: u64,
}

impl Drop for PanelGuard {
    fn drop(&mut self) {
        let id = self.id;
        let _ = PANELS.try_with(|panels| panels.borrow_mut().retain(|(own, _, _)| *own != id));
    }
}

/// How long a segment has to draw before it is worth a line in the trace.
/// `GUINEA_TRACE_RENDER_MS` moves it; `0` records every frame.
fn render_threshold() -> u64 {
    static MICROSECONDS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MICROSECONDS.get_or_init(|| {
        std::env::var("GUINEA_TRACE_RENDER_MS")
            .ok()
            .and_then(|given| given.parse::<f64>().ok())
            .map_or(2_000, |milliseconds| (milliseconds * 1000.0) as u64)
    })
}

/// Times one segment's drawing, and records it if it took long enough to be
/// worth seeing. Sixty frames a second of every page is noise, not a trace.
///
/// The backends put one of these around the call into a page's or layout's
/// own `render`, which is where the application's drawing code runs.
pub struct Rendering {
    segment: &'static str,
    started: std::time::Instant,
    /// Held for as long as the drawing lasts; the profiler reads it on drop.
    #[cfg(feature = "profiling")]
    _zone: Option<puffin::ProfilerScope>,
}

impl Rendering {
    pub fn of(segment: &'static str) -> Option<Rendering> {
        #[cfg(feature = "profiling")]
        {
            let zone = profiling::zone(segment);
            if !trace::is_observed_anywhere() && zone.is_none() {
                return None;
            }

            return Some(Rendering {
                segment,
                started: std::time::Instant::now(),
                _zone: zone,
            });
        }

        #[cfg(not(feature = "profiling"))]
        trace::is_observed_anywhere().then(|| Rendering {
            segment,
            started: std::time::Instant::now(),
        })
    }
}

/// The puffin profiler, when it was compiled in and switched on.
///
/// A profiler is a second reader of the same moments the trace marks: the
/// trace says what happened and why, the profiler says where the frame went.
pub mod profiling {
    /// Whether zones are being recorded. Always `false` without the
    /// `profiling` feature.
    pub fn on() -> bool {
        #[cfg(feature = "profiling")]
        {
            puffin::are_scopes_on()
        }
        #[cfg(not(feature = "profiling"))]
        {
            false
        }
    }

    /// Starts or stops recording. Does nothing without the feature.
    pub fn record(on: bool) {
        #[cfg(feature = "profiling")]
        puffin::set_scopes_on(on);
        #[cfg(not(feature = "profiling"))]
        let _ = on;
    }

    /// Ends the frame the profiler is collecting. A backend calls this once
    /// per frame it draws.
    pub fn frame_done() {
        #[cfg(feature = "profiling")]
        puffin::GlobalProfiler::lock().new_frame();
    }

    /// A `render` zone for as long as the returned value lives, the segment
    /// as its data - which is how puffin carries a name that changes.
    #[cfg(feature = "profiling")]
    pub(super) fn zone(segment: &'static str) -> Option<puffin::ProfilerScope> {
        if !puffin::are_scopes_on() {
            return None;
        }

        static SCOPE: std::sync::OnceLock<puffin::ScopeId> = std::sync::OnceLock::new();
        let scope = *SCOPE.get_or_init(|| {
            puffin::ThreadProfiler::call(|profiler| {
                profiler.register_named_scope("render", "guinea", file!(), line!())
            })
        });

        Some(puffin::ProfilerScope::new(scope, segment))
    }

}

impl Drop for Rendering {
    fn drop(&mut self) {
        let took_us = self.started.elapsed().as_micros() as u64;
        if took_us < render_threshold() {
            return;
        }

        let segment = self.segment;
        trace::mark(|| Point::Render { segment, took_us });
    }
}

/// Marks `point` under what is running now, from any thread.
///
/// On the thread devtools watch, it is marked at once. Anywhere else it is
/// marked there when the UI thread next runs, under the cause that was current
/// here, so a write from background work still leads back to what started it.
pub fn mark_anywhere(point: impl FnOnce() -> Point + Send + 'static) {
    if trace::is_observed() || !trace::is_observed_anywhere() {
        trace::mark(point);
        return;
    }
    let cause = trace::current();
    let marked = crate::actor::try_invoke_on_ui(move || {
        trace::mark_under(cause, point);
    });
    if let Err(unsent) = marked {
        unsent();
    }
}

/// A `tracing` layer that puts the application's own events into the trace,
/// under whatever caused them, while devtools watch.
///
/// Only the application's own: an event written in one of its workspace's
/// crates. guinea's events and every dependency's - winit, wgpu, tokio - stay
/// out of the trace and off the wire; [`LogLayer::all`] lets them in.
///
/// ```no_run
/// use tracing_subscriber::prelude::*;
///
/// tracing_subscriber::registry()
///     .with(tracing_subscriber::fmt::layer())
///     .with(guinea_core::devtools::layer())
///     .init();
/// ```
pub fn layer() -> LogLayer {
    LogLayer { all: false }
}

pub struct LogLayer {
    all: bool,
}

impl LogLayer {
    /// Every event, not only the application's own.
    pub fn all(self) -> Self {
        Self { all: true }
    }
}

/// Whether an event was written in the application's own code.
///
/// Cargo names the files of a workspace member relative to the workspace
/// root, and those of every other crate - registry, git, a path outside the
/// workspace - absolutely. So the application is told apart from its
/// dependencies without a list of either.
fn written_here(file: Option<&str>) -> bool {
    file.is_some_and(|file| std::path::Path::new(file).is_relative())
}

thread_local! {
    static LOGGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        if trace::is_point_target(meta.target())
            || !trace::is_observed_anywhere()
            || LOGGING.get()
            || !(self.all || written_here(meta.file()))
        {
            return;
        }

        LOGGING.set(true);
        let mut text = Text::default();
        event.record(&mut text);

        let (level, target) = (*meta.level(), meta.target());
        let (file, line, module) = (meta.file(), meta.line(), meta.module_path());
        mark_anywhere(move || Point::Log {
            level,
            target,
            file,
            line,
            module,
            text: text.finish(),
        });
        LOGGING.set(false);
    }
}

#[derive(Default)]
struct Text {
    message: String,
    fields: Vec<String>,
}

impl Text {
    fn finish(self) -> String {
        let mut text = self.message;
        for field in self.fields {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&field);
        }
        text
    }
}

impl tracing::field::Visit for Text {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_panel() -> Option<Panel> {
        Some(Panel {
            id: "test",
            title: "Test",
            nodes: Vec::new(),
        })
    }

    #[test]
    fn a_panel_is_offered_for_its_root_until_the_guard_goes() {
        let guard = contribute(7, test_panel);
        assert_eq!(panels(7).len(), 1);
        assert!(panels(8).is_empty());
        assert!(app_panels().is_empty());

        drop(guard);
        assert!(panels(7).is_empty());
    }

    #[cfg(not(feature = "test-utils"))]
    struct Queue(std::sync::mpsc::Sender<crate::actor::UiTask>);

    #[cfg(not(feature = "test-utils"))]
    impl crate::actor::UiDispatcher for Queue {
        fn init(&self) {}

        fn dispatch(&self, task: crate::actor::UiTask) {
            let _ = self.0.send(task);
        }
    }

    #[test]
    #[cfg(not(feature = "test-utils"))]
    fn a_mark_from_another_thread_lands_on_the_watched_one_under_its_cause() {
        let (tx, rx) = std::sync::mpsc::channel();
        crate::actor::set_ui_dispatcher(Queue(tx));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |record| {
            if let trace::Trace::Mark(record) = record {
                sink.borrow_mut().push((record.parent, record.point.kind()));
            }
        });

        let action = trace::mark(|| Point::Action { message: "Save" });
        std::thread::spawn(move || {
            let _resumed = trace::resume(Some(action));
            mark_anywhere(|| Point::Note("written".into()));
        })
        .join()
        .expect("writer");
        for task in rx.try_iter() {
            task();
        }
        trace::stop_observing();

        assert_eq!(
            *seen.borrow(),
            [(None, "action"), (Some(action), "note")],
            "the write is marked here, under the action that caused it"
        );
    }

    #[test]
    fn an_ordinary_event_is_traced_under_what_caused_it() {
        use tracing_subscriber::layer::SubscriberExt;

        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |record| {
            if let trace::Trace::Mark(record) = record
                && let Point::Log { text, .. } = &record.point
            {
                sink.borrow_mut().push((record.parent, text.clone()));
            }
        });

        let subscriber = tracing_subscriber::registry().with(layer());
        let action = trace::mark(|| Point::Action { message: "Kill" });
        tracing::subscriber::with_default(subscriber, || {
            let _resumed = trace::resume(Some(action));
            tracing::info!(pid = 42, "process killed");
            tracing::debug!(target: "guinea::note", "a guinea point, logged: not traced twice");
        });
        trace::stop_observing();

        assert_eq!(
            *seen.borrow(),
            [(Some(action), "process killed pid=42".to_string())]
        );
    }

    #[test]
    fn a_logged_event_says_where_it_was_written() {
        use tracing_subscriber::layer::SubscriberExt;

        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |record| {
            if let trace::Trace::Mark(record) = record
                && let Point::Log {
                    file, line, module, ..
                } = &record.point
            {
                sink.borrow_mut().push((*file, line.is_some(), *module));
            }
        });

        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::with_default(subscriber, || tracing::info!("here"));
        trace::stop_observing();

        assert_eq!(
            *seen.borrow(),
            [(Some(file!()), true, Some(module_path!()))]
        );
    }

    #[test]
    fn only_a_file_of_the_workspace_is_the_applications() {
        assert!(written_here(Some(file!())), "{}", file!());
        assert!(!written_here(Some(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))));
        assert!(!written_here(None));
    }

    #[test]
    fn an_application_panel_belongs_to_no_root() {
        let guard = contribute_to_app(test_panel);
        assert_eq!(app_panels().len(), 1);
        assert!(panels(0).is_empty());

        drop(guard);
        assert!(app_panels().is_empty());
    }
}
