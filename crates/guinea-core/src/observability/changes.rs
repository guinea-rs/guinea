//! What a tool lists came or went: actors, sources, subscriptions, roots.

use std::cell::RefCell;
use std::rc::Rc;

use crate::actor::snapshot::Owner;

/// Something a tool lists came or went, so what it last read of it is
/// stale. Says what to read again, not what it now reads.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Change {
    /// An actor a tool may list came. Which window it is in, if any, is the
    /// window of `owner.scope`.
    ActorAdded {
        id: usize,
        type_name: &'static str,
        owner: Owner,
    },
    /// An actor [`Change::ActorAdded`] told of went. Ids are not reused.
    ActorRemoved { id: usize },
    /// An actor handled a message, so its state may read differently now.
    /// Where it lives is what [`Change::ActorAdded`] said.
    ActorHandled { id: usize },
    /// A source a tool may see opened - a timer is one; its id is the one
    /// its scope lists it under.
    SourceOpened { id: u64 },
    /// A source closed, or stopped being one a tool may see.
    SourceClosed { id: u64 },
    /// Something subscribed to a bus, or stopped being.
    Subscriptions,
    /// A root - a window's tree - opened.
    RootOpened { root: u64 },
    /// A root [`Change::RootOpened`] told of went.
    RootClosed { root: u64 },
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
