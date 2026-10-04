//! Watching a running application from outside it: the public surface a
//! tool - devtools, a test, a logger - reads, and what a plugin offers it.
//!
//! - what happened, and why: [`trace`](crate::trace), with
//!   [`mark_anywhere`] for something off the UI thread;
//! - what came or went: [`changes`];
//! - what a backend or a plugin knows about itself: [`panels`];
//! - where a frame went: [`profiling`];
//! - the application's own `tracing` events: [`layer`].

pub mod changes;
pub mod panels;

use std::cell::RefCell;

use crate::trace::{self, Point};

pub use crate::trace::is_observed;

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

thread_local! {
    static EVERY_RENDER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Records every render on this thread, however quick, while the returned
/// guard lives.
///
/// For a test, which asks which segments a cause redrew. The threshold is
/// noise control for devtools watching a live application, and a test's draw
/// is usually under it. Per thread rather than per process: tests run side by
/// side, and a setting one of them changed would be every other's too.
pub fn record_every_render() -> EveryRender {
    EveryRender(EVERY_RENDER.replace(true))
}

/// Puts the threshold back when dropped. See [`record_every_render`].
pub struct EveryRender(bool);

impl Drop for EveryRender {
    fn drop(&mut self) {
        let _gone = EVERY_RENDER.try_with(|every| every.set(self.0));
    }
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
        if took_us < render_threshold() && !EVERY_RENDER.get() {
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

/// Opens `point` from any thread, with the id [`trace::reserve`] gave, the way
/// [`mark_anywhere`] marks one.
pub(crate) fn begin_anywhere(
    id: trace::Cause,
    parent: Option<trace::Cause>,
    point: impl FnOnce() -> Point + Send + 'static,
) {
    if trace::is_observed() || !trace::is_observed_anywhere() {
        trace::begin_as(id, parent, point);
        return;
    }
    let begun = crate::actor::try_invoke_on_ui(move || trace::begin_as(id, parent, point));
    if let Err(unsent) = begun {
        unsent();
    }
}

/// Closes, from any thread, what [`begin_anywhere`] opened.
pub(crate) fn end_anywhere(id: trace::Cause, took: std::time::Duration) {
    if trace::is_observed() || !trace::is_observed_anywhere() {
        trace::end(id, took);
        return;
    }
    let ended = crate::actor::try_invoke_on_ui(move || trace::end(id, took));
    if let Err(unsent) = ended {
        unsent();
    }
}

/// A `tracing` layer that puts the application's own events and spans into
/// the trace, under whatever caused them, while devtools watch or `guinea::`
/// events are written.
///
/// An event is a [`Point::Log`]. A span is a [`Point::Span`], open from when
/// it is created until it closes and current while it is entered - so what an
/// `#[instrument]`ed function sends, pushes or logs is traced under it - and
/// what it took is the time it spent entered, not the time it waited between
/// polls.
///
/// Only the application's own: an event or span written in one of its
/// workspace's crates. guinea's and every dependency's - winit, wgpu, tokio -
/// stay out of the trace and off the wire; [`LogLayer::all`] lets them in.
///
/// ```no_run
/// use tracing_subscriber::prelude::*;
///
/// tracing_subscriber::registry()
///     .with(tracing_subscriber::fmt::layer())
///     .with(guinea_core::observability::layer())
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

    /// The application's spans entered on this thread, innermost last, each
    /// with the guard that keeps it current and when it was entered.
    static ENTERED: RefCell<Vec<(tracing::span::Id, trace::Resumed, std::time::Instant)>> =
        const { RefCell::new(Vec::new()) };
}

/// A span of the application's own, as the trace knows it: its point, and the
/// time it has spent entered so far.
struct Opened {
    id: trace::Cause,
    busy: std::time::Duration,
}

impl LogLayer {
    fn wants(&self, meta: &tracing::Metadata<'_>) -> bool {
        !trace::is_point_target(meta.target()) && (self.all || written_here(meta.file()))
    }
}

impl<S> tracing_subscriber::Layer<S> for LogLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        cx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = attrs.metadata();
        if !self.wants(meta) || !trace::is_recorded_anywhere() {
            return;
        }
        let Some(span) = cx.span(id) else {
            return;
        };

        let parent = span
            .parent()
            .and_then(|above| above.extensions().get::<Opened>().map(|opened| opened.id))
            .or_else(trace::current);

        let mut fields = Text::default();
        attrs.record(&mut fields);

        let opened = trace::reserve();
        let (name, level, target) = (meta.name(), *meta.level(), meta.target());
        let (file, line, module) = (meta.file(), meta.line(), meta.module_path());
        begin_anywhere(opened, parent, move || Point::Span {
            name,
            level,
            target,
            file,
            line,
            module,
            fields: fields.finish(),
        });

        span.extensions_mut().insert(Opened {
            id: opened,
            busy: std::time::Duration::ZERO,
        });
    }

    fn on_enter(&self, id: &tracing::span::Id, cx: tracing_subscriber::layer::Context<'_, S>) {
        let Some(opened) = cx
            .span(id)
            .and_then(|span| span.extensions().get::<Opened>().map(|opened| opened.id))
        else {
            return;
        };

        let current = trace::resume(Some(opened));
        ENTERED.with(|entered| {
            entered
                .borrow_mut()
                .push((id.clone(), current, std::time::Instant::now()));
        });
    }

    fn on_exit(&self, id: &tracing::span::Id, cx: tracing_subscriber::layer::Context<'_, S>) {
        let left = ENTERED.with(|entered| {
            let mut entered = entered.borrow_mut();
            let at = entered.iter().rposition(|(span, ..)| span == id)?;
            Some(entered.remove(at))
        });
        let Some((_, current, since)) = left else {
            return;
        };
        drop(current);

        if let Some(span) = cx.span(id)
            && let Some(opened) = span.extensions_mut().get_mut::<Opened>()
        {
            opened.busy += since.elapsed();
        }
    }

    fn on_close(&self, id: tracing::span::Id, cx: tracing_subscriber::layer::Context<'_, S>) {
        let Some(opened) = cx
            .span(&id)
            .and_then(|span| span.extensions_mut().remove::<Opened>())
        else {
            return;
        };

        end_anywhere(opened.id, opened.busy);
    }

    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        if !self.wants(meta) || !trace::is_observed_anywhere() || LOGGING.get() {
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
    use std::rc::Rc;

    use super::changes::{Change, changed};
    use super::panels::{Panel, contribute, contribute_to_app, for_app, for_root};
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
        assert_eq!(for_root(7).len(), 1);
        assert!(for_root(8).is_empty());
        assert!(for_app().is_empty());

        drop(guard);
        assert!(for_root(7).is_empty());
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
    fn a_span_is_open_under_what_caused_it_and_current_while_entered() {
        use tracing_subscriber::layer::SubscriberExt;

        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |trace| sink.borrow_mut().push(trace.clone()));

        let subscriber = tracing_subscriber::registry().with(layer());
        let action = trace::mark(|| Point::Action { message: "Refresh" });
        tracing::subscriber::with_default(subscriber, || {
            let _resumed = trace::resume(Some(action));
            let span = tracing::info_span!("rows_from_report", rows = 3);
            {
                let _entered = span.enter();
                trace::mark(|| Point::Push { reducer: "Rows" });
            }
            trace::mark(|| Point::Note("after the span".into()));
        });
        trace::stop_observing();

        let seen = seen.borrow();
        let span = seen
            .iter()
            .find_map(|trace| match trace {
                trace::Trace::Begin(record) => Some(record),
                _ => None,
            })
            .expect("the span was opened");
        assert_eq!(span.parent, Some(action));
        assert!(
            matches!(
                &span.point,
                Point::Span { name: "rows_from_report", fields, file: Some(file), .. }
                    if fields == "rows=3" && *file == file!()
            ),
            "{:?}",
            span.point
        );

        let parent_of = |kind: &str| {
            seen.iter().find_map(|trace| match trace {
                trace::Trace::Mark(record) if record.point.kind() == kind => Some(record.parent),
                _ => None,
            })
        };
        assert_eq!(parent_of("push"), Some(Some(span.id)), "inside it, under it");
        assert_eq!(parent_of("note"), Some(Some(action)), "after it, under what was there");
        assert!(
            matches!(seen.last(), Some(trace::Trace::End { id, .. }) if *id == span.id),
            "{seen:?}"
        );
    }

    #[test]
    fn nothing_is_built_for_a_change_nobody_watches() {
        let built = std::cell::Cell::new(false);
        changed(|| {
            built.set(true);
            Change::TimerStarted { id: 0 }
        });

        assert!(!built.get());
    }

    #[test]
    fn a_span_is_at_the_level_it_was_opened_at() {
        use tracing_subscriber::layer::SubscriberExt;

        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |trace| {
            if let trace::Trace::Begin(record) = trace
                && let Point::Span { name, level, .. } = &record.point
            {
                sink.borrow_mut().push((*name, *level));
            }
        });

        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::with_default(subscriber, || {
            let _warned = tracing::warn_span!("retrying").entered();
            let _debugged = tracing::debug_span!("parsing").entered();
        });
        trace::stop_observing();

        assert_eq!(
            *seen.borrow(),
            [("retrying", tracing::Level::WARN), ("parsing", tracing::Level::DEBUG)]
        );
    }

    #[test]
    fn a_span_took_the_time_it_was_entered_not_the_time_between() {
        use tracing_subscriber::layer::SubscriberExt;

        let took = Rc::new(RefCell::new(Vec::new()));
        let sink = took.clone();
        trace::observe(move |trace| {
            if let trace::Trace::End { took, .. } = trace {
                sink.borrow_mut().push(*took);
            }
        });

        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("polled");
            for _ in 0..2 {
                let _entered = span.enter();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        });
        trace::stop_observing();

        let took = *took.borrow().first().expect("the span closed");
        assert!(
            took >= std::time::Duration::from_millis(10) && took < std::time::Duration::from_millis(40),
            "two entries of five milliseconds, forty idle after: {took:?}"
        );
    }

    #[test]
    fn a_span_written_elsewhere_is_not_traced() {
        use tracing_subscriber::layer::SubscriberExt;

        let seen = Rc::new(RefCell::new(0));
        let sink = seen.clone();
        trace::observe(move |_| *sink.borrow_mut() += 1);

        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::span!(target: "guinea::handle", tracing::Level::INFO, "a guinea point");
            let _entered = span.enter();
        });
        trace::stop_observing();

        assert_eq!(*seen.borrow(), 0);
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
        assert_eq!(for_app().len(), 1);
        assert!(for_root(0).is_empty());

        drop(guard);
        assert!(for_app().is_empty());
    }
}
