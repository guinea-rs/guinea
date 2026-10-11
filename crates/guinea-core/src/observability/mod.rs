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
mod log_layer;
pub mod panels;

use crate::trace::{self, Point};

pub use log_layer::{LogLayer, layer};

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

#[cfg(test)]
mod tests {
    use super::changes::{Change, changed};
    use super::panels::{Panel, contribute, contribute_to_app, for_app, for_root};

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
        use std::cell::RefCell;
        use std::rc::Rc;

        use super::*;

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
    fn nothing_is_built_for_a_change_nobody_watches() {
        let built = std::cell::Cell::new(false);
        changed(|| {
            built.set(true);
            Change::TimerStarted { id: 0 }
        });

        assert!(!built.get());
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
