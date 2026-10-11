//! Getting background work back onto the thread that draws.
//!
//! WinUI hands this to the system: its dispatcher queue belongs to the UI
//! thread and the reactor pumps it. A terminal has no such queue - the loop in
//! [`crate::run`] *is* the pump - so this is a plain channel that the loop
//! drains between frames.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};

use guinea_core::ui_thread::{UiDispatcher, UiTask, set_ui_dispatcher};

struct ChannelDispatcher(Sender<UiTask>);

impl UiDispatcher for ChannelDispatcher {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        // A send that fails means the loop is gone, which happens only while
        // the process is shutting down. The work is moot by then.
        let _ = self.0.send(task);
    }
}

thread_local! {
    /// The receiving end belongs to whichever thread draws - it is the one
    /// that runs the tasks, and a `Receiver` is not `Sync` anyway.
    static TASKS: RefCell<Option<Receiver<UiTask>>> = const { RefCell::new(None) };
}

/// Installs the dispatcher. Call once, from the drawing thread, before any
/// actor exists.
pub(crate) fn install() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let (tx, rx) = channel();
        TASKS.with(|slot| *slot.borrow_mut() = Some(rx));
        set_ui_dispatcher(ChannelDispatcher(tx));
    });
}

/// Runs whatever actors queued since the last frame, and reports whether
/// anything ran - a frame after work is a frame worth redrawing.
pub(crate) fn drain() -> bool {
    // Collected before running any of them: a task is free to queue another,
    // and draining into the next frame keeps one busy actor from starving the
    // screen.
    let ready: Vec<UiTask> = TASKS.with(|slot| match slot.borrow().as_ref() {
        // `try_iter` and not `iter`: this runs between frames and must not
        // block waiting for work that may never come.
        Some(rx) => rx.try_iter().collect(),
        None => Vec::new(),
    });

    let ran = !ready.is_empty();
    for task in ready {
        task();
    }
    ran
}
