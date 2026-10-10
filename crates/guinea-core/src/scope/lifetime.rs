use std::rc::Rc;

use tokio::task::JoinHandle;

use crate::actor::event_bus::subscribe::BusSubscription;

use super::{Scope, ScopeData};

/// Whether a scope is awake, for something it owns to ask on its own.
///
/// Held rather than the scope itself, so that a timer the scope owns can ask
/// from off the scope's own code paths.
#[derive(Clone)]
pub struct Awake(Rc<std::cell::Cell<bool>>);

impl Awake {
    /// Whether the scope is awake now.
    pub fn now(&self) -> bool {
        !self.0.get()
    }
}

impl Scope {
    pub fn own_subscription(&self, subscription: BusSubscription) {
        self.own(DropGuard(subscription));
    }

    /// Binds any [`Teardown`] resource to this scope's lifetime: it is torn
    /// down when the scope is - at once, if the scope is already gone.
    pub fn own<R: Teardown>(&self, resource: R) {
        match self.data() {
            Some(data) => data
                .teardowns
                .borrow_mut()
                .push(Box::new(move || resource.teardown())),
            None => resource.teardown(),
        }
    }

    /// Asked before this scope is torn down by a navigation.
    ///
    /// Registered during `install`, which is the whole reason leaving and
    /// entering are declared in different places: on the way out the scope
    /// exists, so the guard can read its own state - which is what "unsaved
    /// changes" is. On the way in there is nothing to read yet.
    pub fn on_leave(&self, guard: impl Fn() -> crate::guard::Verdict + 'static) {
        let data = self.installing("guarding a leave");
        data.leave_guards.borrow_mut().push(Rc::new(guard));
    }

    /// The guards to ask, in the order they were registered.
    ///
    /// Cloned out rather than borrowed: a guard is free to touch this scope,
    /// and the caller runs them while deciding.
    pub fn leave_guards(&self) -> Vec<Rc<dyn Fn() -> crate::guard::Verdict>> {
        self.data()
            .map(|data| data.leave_guards.borrow().clone())
            .unwrap_or_default()
    }

    /// Puts this scope to sleep: kept, with its state and actors, but deaf.
    ///
    /// What it owns stops hearing anything while it sleeps - its timers skip
    /// their ticks, its actors and callbacks are not told what the buses
    /// carry - and nothing is queued for later: what happened meanwhile is
    /// missed. A router does this to a `keep` segment it leaves.
    pub fn sleep(&self) {
        if let Some(data) = self.data() {
            data.asleep.set(true);
        }
    }

    /// Wakes this scope, then runs what asked to hear of it, in the order it
    /// asked.
    pub fn wake(&self) {
        let Some(data) = self.data() else { return };
        data.asleep.set(false);

        let hooks = data.wake_hooks.borrow().clone();
        drop(data);
        for hook in hooks {
            hook();
        }
    }

    /// Whether this scope is there and awake. A removed one is neither.
    pub fn is_awake(&self) -> bool {
        self.data().is_some_and(|data| !data.asleep.get())
    }

    /// Whether this scope is awake, for what it owns to ask later.
    pub fn awake(&self) -> Awake {
        match self.data() {
            Some(data) => Awake(data.asleep.clone()),
            None => Awake(Rc::new(std::cell::Cell::new(true))),
        }
    }

    /// Runs `hook` every time this scope wakes: for a feature to catch up on
    /// what it missed while it slept.
    pub fn on_wake(&self, hook: impl Fn() + 'static) {
        let data = self.installing("waiting for a wake");
        data.wake_hooks.borrow_mut().push(Rc::new(hook));
    }
}

impl ScopeData {
    /// Lets go of what this scope took on, the last first: what came later
    /// may rest on what came before it.
    pub(super) fn tear_down(&self) {
        let teardowns = std::mem::take(&mut *self.teardowns.borrow_mut());
        for teardown in teardowns.into_iter().rev() {
            teardown();
        }
    }
}

/// A resource whose lifetime can be bound to a [`Scope`] via [`Scope::own`].
/// Implement per resource kind; the "blanket" over arbitrary `T` is the
/// [`DropGuard`] newtype, so specialized teardowns never overlap.
pub trait Teardown: 'static {
    fn teardown(self);
}

impl Teardown for JoinHandle<()> {
    fn teardown(self) {
        self.abort();
    }
}

/// Blanket teardown for resources that just need dropping
/// (`scope.own(DropGuard(resource))` when no specialized impl exists).
pub struct DropGuard<T: 'static>(pub T);

impl<T: 'static> Teardown for DropGuard<T> {
    fn teardown(self) {
        drop(self.0);
    }
}
