//! What a tool lists came or went: actors, timers, subscriptions, routers.

use std::cell::RefCell;
use std::rc::Rc;

use crate::actor::registry::Owner;
use crate::trace;

/// Something a tool lists came or went, so what it last read of it is
/// stale. Says what to read again, not what it now reads.
///
/// `root` is the window a registry or a bus belongs to, as `RootId::get`
/// numbers it; `None` for the application's own, or for one no window owns.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Change {
    ActorAdded {
        root: Option<u64>,
        id: usize,
        type_name: &'static str,
        owner: Owner,
    },
    ActorRemoved {
        root: Option<u64>,
        id: usize,
    },
    /// An actor handled a message, so its state may read differently now.
    /// Where it lives is what [`Change::ActorAdded`] said.
    ActorHandled { id: usize },
    /// A timer a tool may see started; its id is the one the running timers
    /// list it under.
    TimerStarted { id: u64 },
    /// A timer stopped, or stopped being one a tool may see.
    TimerStopped { id: u64 },
    /// Something subscribed to a bus, or stopped being.
    Subscriptions { bus: trace::Bus, root: Option<u64> },
    /// A window's router navigated for the first time.
    RouterOpened { root: u64 },
    RouterClosed { root: u64 },
}

type Watcher = Rc<dyn Fn(&Change)>;

thread_local! {
    static WATCHER: RefCell<Option<Watcher>> = const { RefCell::new(None) };
}

/// Hands every [`Change`] on this thread to `watcher`, replacing any watcher
/// already set.
///
/// It is told synchronously, from wherever the change happened - a
/// registration, a teardown - so it should note what to read again and read
/// it later, not read it there.
pub fn watch(watcher: impl Fn(&Change) + 'static) {
    WATCHER.with(|slot| slot.borrow_mut().replace(Rc::new(watcher)));
}

pub fn stop_watching() {
    WATCHER.with(|slot| slot.borrow_mut().take());
}

/// Tells the watcher, if there is one; `change` is not built otherwise.
pub fn changed(change: impl FnOnce() -> Change) {
    let Some(watcher) = WATCHER.try_with(|slot| slot.borrow().clone()).ok().flatten() else {
        return;
    };

    watcher(&change());
}
