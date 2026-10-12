use std::cell::RefCell;
use std::rc::Rc;

use super::Scope;

/// What a scope asks before it goes, and whether it is awake meanwhile.
#[derive(Default)]
pub(super) struct Lifetime {
    /// Asked before the scope is torn down. See [`Scope::on_leave`].
    leave_guards: RefCell<Vec<Rc<dyn Fn() -> crate::guard::Verdict>>>,
    /// Set while a router keeps the scope without showing it. See
    /// [`Scope::sleep`].
    asleep: Rc<std::cell::Cell<bool>>,
    /// Run when it is shown again. See [`Scope::on_wake`].
    wake_hooks: RefCell<Vec<Rc<dyn Fn()>>>,
}

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
    /// Asked before this scope is torn down by a navigation.
    ///
    /// Registered during `install`, which is the whole reason leaving and
    /// entering are declared in different places: on the way out the scope
    /// exists, so the guard can read its own state - which is what "unsaved
    /// changes" is. On the way in there is nothing to read yet.
    pub fn on_leave(&self, guard: impl Fn() -> crate::guard::Verdict + 'static) {
        let data = self.alive("guarding a leave");
        data.lifetime.leave_guards.borrow_mut().push(Rc::new(guard));
    }

    /// The guards to ask, in the order they were registered.
    ///
    /// Cloned out rather than borrowed: a guard is free to touch this scope,
    /// and the caller runs them while deciding.
    pub fn leave_guards(&self) -> Vec<Rc<dyn Fn() -> crate::guard::Verdict>> {
        self.data()
            .map(|data| data.lifetime.leave_guards.borrow().clone())
            .unwrap_or_default()
    }

    /// Puts this scope to sleep: kept, with its state and actors, but deaf.
    ///
    /// What it owns stops hearing anything while it sleeps - its timers skip
    /// their ticks, its actors and callbacks are not told what the buses
    /// carry - and nothing is queued for later: what happened meanwhile is
    /// missed. A router does this to a `keep` segment it leaves.
    pub fn sleep(&self) {
        self.alive("putting to sleep").lifetime.asleep.set(true);
    }

    /// Wakes this scope, then runs what asked to hear of it, in the order it
    /// asked - until one of them removes the scope or puts it back to sleep.
    pub fn wake(&self) {
        let data = self.alive("waking");
        data.lifetime.asleep.set(false);

        let hooks = data.lifetime.wake_hooks.borrow().clone();
        drop(data);
        for hook in hooks {
            if !self.is_awake() {
                break;
            }

            hook();
        }
    }

    /// Whether this scope is there and awake. A removed one is neither.
    pub fn is_awake(&self) -> bool {
        self.data().is_some_and(|data| !data.lifetime.asleep.get())
    }

    /// Whether this scope is awake, for what it owns to ask later.
    pub fn awake(&self) -> Awake {
        match self.data() {
            Some(data) => Awake(data.lifetime.asleep.clone()),
            None => Awake(Rc::new(std::cell::Cell::new(true))),
        }
    }

    /// Runs `hook` every time this scope wakes: for a feature to catch up on
    /// what it missed while it slept.
    pub fn on_wake(&self, hook: impl Fn() + 'static) {
        let data = self.alive("waiting for a wake");
        data.lifetime.wake_hooks.borrow_mut().push(Rc::new(hook));
    }
}
