//! Getting background work back onto the thread that owns the window.
//!
//! Slint has a queue of its own, so unlike the terminal backend there is
//! nothing to pump here: `invoke_from_event_loop` posts to the loop the
//! application is already running.

use guinea_core::ui_thread::{UiDispatcher, UiTask, set_ui_dispatcher};

struct EventLoopDispatcher;

impl UiDispatcher for EventLoopDispatcher {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        // Fails only when the loop has not started yet or has already
        // finished. Neither is worth failing an actor over: before the loop
        // there is no window to update, and after it the process is on its way
        // out.
        if let Err(e) = slint::invoke_from_event_loop(task) {
            tracing::debug!(error = %e, "dropped a task queued for the event loop");
        }
    }
}

/// Installs the dispatcher. Call once, from the thread that owns the window,
/// before any actor exists.
pub(crate) fn install() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| set_ui_dispatcher(EventLoopDispatcher));
}
