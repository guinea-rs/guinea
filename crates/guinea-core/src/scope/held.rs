use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::Location;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::actor::event_bus::subscribe::BusSubscription;
use crate::feed::Opened;
use crate::sources::Pouring;

use super::Scope;
use super::actors::HeldActor;
use super::state::Subscription;

static NEXT_KEY: AtomicUsize = AtomicUsize::new(1);

/// The key for the next thing a scope takes on. Keys only grow, so they say
/// in what order things were taken; an actor's is its id.
pub(crate) fn next_key() -> usize {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

/// What a scope took on, by when it took it, until it lets go of it.
#[derive(Default)]
pub(super) struct Held(pub(super) RefCell<BTreeMap<usize, Taken>>);

/// One thing a scope takes on.
pub(super) enum Taken {
    /// An actor it owns, under its id.
    Actor(HeldActor),
    /// Something heard on the bus of its tree.
    Hearing(BusSubscription),
    /// A reducer watched.
    Watching(Subscription),
    /// A source it pours. See [`crate::sources`].
    Source(Pouring),
    /// A feed it opened. See [`crate::feed`].
    Feed(Rc<dyn Opened>),
    /// Run when it goes.
    Cleanup(Box<dyn FnOnce()>),
}

impl Taken {
    fn let_go(self, key: usize) {
        match self {
            Taken::Actor(actor) => actor.let_go(key),
            Taken::Hearing(hearing) => drop(hearing),
            Taken::Watching(watching) => drop(watching),
            Taken::Source(pouring) => drop(pouring),
            Taken::Feed(fed) => fed.close(),
            Taken::Cleanup(run) => run(),
        }
    }
}

impl Held {
    /// Lets go of everything, the last taken first: what came later may rest
    /// on what came before it.
    pub(super) fn let_go(&self) {
        loop {
            let last = self.0.borrow_mut().pop_last();
            let Some((key, taken)) = last else { break };
            taken.let_go(key);
        }
    }
}

impl Scope {
    /// Takes `taken` on under `key` - or lets go of it at once, when this
    /// scope is already gone.
    pub(super) fn take(&self, key: usize, taken: Taken) {
        match self.data() {
            Some(data) => {
                data.held.0.borrow_mut().insert(key, taken);
            }
            None => taken.let_go(key),
        }
    }

    /// Hears what `hearing` subscribed to for as long as this scope is there.
    pub fn hold_hearing(&self, hearing: BusSubscription) {
        self.take(next_key(), Taken::Hearing(hearing));
    }

    /// Watches what `watching` subscribed to for as long as this scope is
    /// there.
    pub fn hold_watching(&self, watching: Subscription) {
        self.take(next_key(), Taken::Watching(watching));
    }

    /// Pours the source `pouring` under `key` until this scope goes, or the
    /// source ends.
    ///
    /// # Panics
    ///
    /// When this scope is already gone.
    pub(crate) fn hold_source(&self, key: usize, pouring: Pouring) {
        self.alive("opening a source");
        self.take(key, Taken::Source(pouring));
    }

    /// Reads the source this scope holds under `key`, with the table
    /// borrowed only for as long as `read` takes.
    pub(crate) fn held_source<R>(&self, key: usize, read: impl FnOnce(&Pouring) -> R) -> Option<R> {
        let data = self.data()?;
        let held = data.held.0.borrow();

        match held.get(&key)? {
            Taken::Source(pouring) => Some(read(pouring)),
            _ => None,
        }
    }

    /// Reads every source this scope holds itself, oldest first.
    pub(crate) fn each_source(&self, mut read: impl FnMut(&Pouring)) {
        let Some(data) = self.data() else {
            return;
        };

        for taken in data.held.0.borrow().values() {
            if let Taken::Source(pouring) = taken {
                read(pouring);
            }
        }
    }

    /// Lets go of what this scope holds under `key`, if it still holds it.
    pub(crate) fn let_go_of(&self, key: usize) {
        let Some(data) = self.data() else {
            return;
        };

        let taken = data.held.0.borrow_mut().remove(&key);
        drop(data);

        if let Some(taken) = taken {
            taken.let_go(key);
        }
    }

    /// Holds the feed `fed` under `key`, or names where the one of its type
    /// this scope already holds was opened.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub(crate) fn hold_feed(
        &self,
        key: usize,
        fed: Rc<dyn Opened>,
    ) -> Result<(), &'static Location<'static>> {
        self.alive("opening a feed");

        if let Some(first) = self.feed_key(fed.of()).and_then(|first| self.held_opened(first)) {
            return Err(first.opened());
        }
        self.take(key, Taken::Feed(fed));
        Ok(())
    }

    /// The key of the feed of the type `of` this scope opened, if it opened
    /// one.
    pub(crate) fn feed_key(&self, of: TypeId) -> Option<usize> {
        let data = self.data()?;
        let held = data.held.0.borrow();

        held.iter().find_map(|(key, taken)| match taken {
            Taken::Feed(fed) if fed.of() == of => Some(*key),
            _ => None,
        })
    }

    /// The feed this scope holds under `key`.
    pub(crate) fn held_feed(&self, key: usize) -> Option<Rc<dyn Any>> {
        self.held_opened(key).map(Opened::into_any)
    }

    fn held_opened(&self, key: usize) -> Option<Rc<dyn Opened>> {
        let data = self.data()?;
        let held = data.held.0.borrow();

        match held.get(&key)? {
            Taken::Feed(fed) => Some(fed.clone()),
            _ => None,
        }
    }

    /// Runs `run` when this scope goes, in its turn among what it took on -
    /// at once, when it is already gone.
    pub fn on_teardown(&self, run: impl FnOnce() + 'static) {
        self.take(next_key(), Taken::Cleanup(Box::new(run)));
    }
}
