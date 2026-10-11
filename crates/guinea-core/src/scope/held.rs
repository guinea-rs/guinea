use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::actor::event_bus::subscribe::BusSubscription;
use crate::timers::Ticking;

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
    /// What a feature installed in it handed back.
    Installed(Box<dyn Any>),
    Timer(Ticking),
    /// Run when it goes.
    Cleanup(Box<dyn FnOnce()>),
}

impl Taken {
    fn let_go(self, key: usize) {
        match self {
            Taken::Actor(actor) => actor.let_go(key),
            Taken::Hearing(hearing) => drop(hearing),
            Taken::Watching(watching) => drop(watching),
            Taken::Installed(installed) => drop(installed),
            Taken::Timer(ticking) => drop(ticking),
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

    /// Keeps what a feature's `install` handed back for as long as this scope
    /// is there.
    pub fn hold_installed<T: 'static>(&self, installed: T) {
        self.take(next_key(), Taken::Installed(Box::new(installed)));
    }

    pub(crate) fn hold_timer(&self, ticking: Ticking) {
        self.take(next_key(), Taken::Timer(ticking));
    }

    /// Runs `run` when this scope goes, in its turn among what it took on -
    /// at once, when it is already gone.
    pub fn on_teardown(&self, run: impl FnOnce() + 'static) {
        self.take(next_key(), Taken::Cleanup(Box::new(run)));
    }
}
