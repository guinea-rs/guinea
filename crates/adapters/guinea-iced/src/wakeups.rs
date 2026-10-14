//! Getting iced to ask for a turn when work for the UI thread arrives.
//!
//! The work itself waits in a [`UiLoop`], as in every backend that owns no
//! queue of its own. What is different about iced is that there is no place
//! to reach in and drain. A terminal polls, egui hands over the top of each
//! frame; iced only ever calls `update`, and only with a message.
//!
//! So the wake-up has to be a message. A subscription streams one in whenever
//! the loop is woken, and `update` drains the loop on every message. Only the
//! token crosses threads, never the work; the loop wakes once for a burst, so
//! a hundred actor messages cost one turn.

use std::sync::{Mutex, OnceLock};

use guinea_core::ui_thread::UiLoop;
use iced::futures::StreamExt;
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::futures::stream::{Stream, pending};

use crate::Envelope;

fn wakeups_out() -> &'static Mutex<Option<UnboundedSender<()>>> {
    static SENDER: OnceLock<Mutex<Option<UnboundedSender<()>>>> = OnceLock::new();
    SENDER.get_or_init(|| Mutex::new(None))
}

fn wakeups_in() -> &'static Mutex<Option<UnboundedReceiver<()>>> {
    static RECEIVER: OnceLock<Mutex<Option<UnboundedReceiver<()>>>> = OnceLock::new();
    RECEIVER.get_or_init(|| Mutex::new(None))
}

/// Makes this thread the UI thread, its work waiting in the loop returned
/// and announced through [`wakeups`]. Call from the drawing thread, before
/// any actor exists.
pub(crate) fn install() -> UiLoop {
    let (wake_tx, wake_rx) = unbounded();
    if let Ok(mut slot) = wakeups_out().lock() {
        *slot = Some(wake_tx);
    }
    if let Ok(mut slot) = wakeups_in().lock() {
        *slot = Some(wake_rx);
    }

    UiLoop::install_waking(|| {
        if let Ok(sender) = wakeups_out().lock()
            && let Some(sender) = sender.as_ref()
        {
            let _ = sender.unbounded_send(());
        }
    })
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

/// Stops waking a loop that is no longer there.
pub(crate) fn close() {
    if let Ok(mut sender) = wakeups_out().lock() {
        *sender = None;
    }
}
