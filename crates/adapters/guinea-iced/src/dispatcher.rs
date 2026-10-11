//! Getting background work back onto the thread that draws - and getting iced
//! to ask for it.
//!
//! Every backend here has the same two halves: a queue that carries work to
//! the UI thread, and a way to wake the loop that drains it. What is different
//! about iced is that there is no place to reach in and drain. A terminal
//! polls, egui hands over the top of each frame, Slint has a rendering
//! notifier; iced only ever calls `update`, and only with a message.
//!
//! So the wake-up has to be a message. A subscription streams one in whenever
//! a worker queues something, `update` recognises it as nobody's message in
//! particular and drains the queue. That is the integration point, and it is
//! also why the queue itself stays thread-local: only the token crosses
//! threads, never the work.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use guinea_core::ui_thread::{UiDispatcher, UiTask, set_ui_dispatcher};
use iced::futures::StreamExt;
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::futures::stream::{Stream, pending};

use crate::Envelope;

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

/// Whether a wake-up is already on its way.
///
/// Without this a burst of a hundred actor messages is a hundred updates, each
/// one drawing a frame to find the queue already empty. With it the burst
/// costs one token, and the queue - which is drained whole - carries the rest.
static PENDING_WAKE: AtomicBool = AtomicBool::new(false);

fn wakeups_out() -> &'static Mutex<Option<UnboundedSender<()>>> {
    static SENDER: OnceLock<Mutex<Option<UnboundedSender<()>>>> = OnceLock::new();
    SENDER.get_or_init(|| Mutex::new(None))
}

fn wakeups_in() -> &'static Mutex<Option<UnboundedReceiver<()>>> {
    static RECEIVER: OnceLock<Mutex<Option<UnboundedReceiver<()>>>> = OnceLock::new();
    RECEIVER.get_or_init(|| Mutex::new(None))
}

/// Installs the dispatcher. Call once, from the drawing thread, before any
/// actor exists.
pub(crate) fn install() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let (tx, rx) = channel();
        TASKS.with(|slot| *slot.borrow_mut() = Some(rx));

        let (wake_tx, wake_rx) = unbounded();
        if let Ok(mut slot) = wakeups_out().lock() {
            *slot = Some(wake_tx);
        }
        if let Ok(mut slot) = wakeups_in().lock() {
            *slot = Some(wake_rx);
        }

        set_ui_dispatcher(ChannelDispatcher(tx));
    });
}

fn wake() {
    if PENDING_WAKE.swap(true, Ordering::SeqCst) {
        return;
    }

    if let Ok(sender) = wakeups_out().lock()
        && let Some(sender) = sender.as_ref()
    {
        let _ = sender.unbounded_send(());
    }
}

/// The stream iced subscribes to. Takes the receiving end, so a second
/// subscription - which iced does not make - would go quiet rather than steal
/// wake-ups from the first.
pub(crate) fn wakeups() -> impl Stream<Item = Envelope> + Send + 'static {
    let taken = wakeups_in().lock().ok().and_then(|mut slot| slot.take());

    match taken {
        Some(receiver) => receiver.map(|()| Envelope::settled()).left_stream(),
        None => pending().right_stream(),
    }
}

/// Runs whatever actors queued since the last update.
pub(crate) fn drain() {
    // Disarmed before the queue is read, not after: work queued while these
    // tasks run must produce a new wake-up rather than be lost behind a flag
    // that is still set.
    PENDING_WAKE.store(false, Ordering::SeqCst);

    // Collected before running any of them: a task is free to queue another,
    // and draining into the next turn keeps one busy actor from starving the
    // screen.
    let ready: Vec<UiTask> = TASKS.with(|slot| match slot.borrow().as_ref() {
        Some(rx) => rx.try_iter().collect(),
        None => Vec::new(),
    });

    for task in ready {
        task();
    }
}

/// Stops waking a loop that is no longer there.
pub(crate) fn close() {
    if let Ok(mut sender) = wakeups_out().lock() {
        *sender = None;
    }
}
