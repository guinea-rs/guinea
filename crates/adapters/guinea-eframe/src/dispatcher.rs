//! Getting background work back onto the thread that draws - and waking it.
//!
//! Two halves, unlike the other backends. The queue is a channel drained
//! between frames, as in the terminal. The wake-up is the part egui adds:
//! it sleeps when nothing happens, so a task queued from a worker thread
//! would sit there until the user happened to move the mouse.
//! `Context::request_repaint` is what asks for one more frame, and it is
//! `Send`, which is the whole reason this works.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use guinea_core::ui_thread::{UiDispatcher, UiTask, set_ui_dispatcher};

struct ChannelDispatcher(Sender<UiTask>);

impl UiDispatcher for ChannelDispatcher {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        // A send that fails means the loop is gone, which happens only while
        // the process is shutting down. The work is moot by then.
        if self.0.send(task).is_ok() {
            wake();
        }
    }
}

thread_local! {
    /// The receiving end belongs to whichever thread draws - it is the one
    /// that runs the tasks, and a `Receiver` is not `Sync` anyway.
    static TASKS: RefCell<Option<Receiver<UiTask>>> = const { RefCell::new(None) };
}

fn waker() -> &'static Mutex<Option<egui::Context>> {
    static WAKER: OnceLock<Mutex<Option<egui::Context>>> = OnceLock::new();
    WAKER.get_or_init(|| Mutex::new(None))
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

/// Hands over the context to wake, once eframe has created one.
pub(crate) fn wake_with(context: egui::Context) {
    if let Ok(mut waker) = waker().lock() {
        *waker = Some(context);
    }
}

pub(crate) fn forget_waker() {
    if let Ok(mut waker) = waker().lock() {
        *waker = None;
    }
}

/// Asks for a frame. Called from whichever thread queued the work.
fn wake() {
    if let Ok(waker) = waker().lock()
        && let Some(context) = waker.as_ref()
    {
        context.request_repaint();
    }
}

/// Runs whatever actors queued since the last frame, and reports whether
/// anything ran - a frame after work is a frame worth drawing again.
pub(crate) fn drain() -> bool {
    // Collected before running any of them: a task is free to queue another,
    // and draining into the next frame keeps one busy actor from starving the
    // screen.
    let ready: Vec<UiTask> = TASKS.with(|slot| match slot.borrow().as_ref() {
        Some(rx) => rx.try_iter().collect(),
        None => Vec::new(),
    });

    let ran = !ready.is_empty();
    for task in ready {
        task();
    }
    ran
}
