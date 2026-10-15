use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use super::Scope;
use crate::observability::observer::{self, Event};

/// What a scope asks before it goes, and whether it is awake meanwhile.
#[derive(Default)]
pub(super) struct Lifetime {
    /// Asked before the scope is torn down. See [`Scope::on_leave`].
    leave_guards: RefCell<Vec<Rc<dyn Fn() -> crate::guard::Verdict>>>,
    /// Set while a router keeps the scope without showing it. See
    /// [`Scope::sleep`].
    asleep: Arc<Gate>,
    /// Run when it is shown again. See [`Scope::on_wake`].
    wake_hooks: RefCell<Vec<Rc<dyn Fn()>>>,
    /// Run when it is put to sleep. See [`Scope::on_sleep`].
    sleep_hooks: RefCell<Vec<Rc<dyn Fn()>>>,
}

#[derive(Default)]
struct Gate {
    asleep: AtomicBool,
    waiting: Mutex<Vec<Waker>>,
}

impl Gate {
    fn asleep() -> Self {
        Self {
            asleep: AtomicBool::new(true),
            waiting: Mutex::default(),
        }
    }

    fn set(&self, asleep: bool) {
        self.asleep.store(asleep, Ordering::SeqCst);

        if !asleep {
            let waiting = std::mem::take(&mut *self.waiting.lock());
            waiting.into_iter().for_each(Waker::wake);
        }
    }

    fn is_asleep(&self) -> bool {
        self.asleep.load(Ordering::SeqCst)
    }
}

/// Whether a scope is awake, for something it owns to ask on its own - from
/// any thread.
///
/// Held rather than the scope itself, so that a source the scope owns can
/// ask from off the scope's own code paths, and wait for it to wake.
#[derive(Clone)]
pub struct Awake(Arc<Gate>);

impl Awake {
    /// Whether the scope is awake now.
    pub fn now(&self) -> bool {
        !self.0.is_asleep()
    }

    /// Ready once the scope is awake: at once when it is. A removed scope
    /// never wakes.
    pub fn wait(&self) -> Waiting {
        Waiting(self.0.clone())
    }
}

/// What [`Awake::wait`] returns.
pub struct Waiting(Arc<Gate>);

impl Future for Waiting {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if !self.0.is_asleep() {
            return Poll::Ready(());
        }

        self.0.waiting.lock().push(cx.waker().clone());

        match self.0.is_asleep() {
            true => Poll::Pending,
            false => Poll::Ready(()),
        }
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
    ///
    /// Then runs what asked to hear of it, in the order it asked - until one
    /// of them removes the scope or wakes it.
    pub fn sleep(&self) {
        let data = self.alive("putting to sleep");
        if !data.lifetime.asleep.is_asleep() {
            observer::tell(*self, || Event::Slept { scope: self.id() });
        }
        data.lifetime.asleep.set(true);

        let hooks = data.lifetime.sleep_hooks.borrow().clone();
        drop(data);
        for hook in hooks {
            if !self.is_alive() || self.is_awake() {
                break;
            }

            hook();
        }
    }

    /// Wakes this scope, then runs what asked to hear of it, in the order it
    /// asked - until one of them removes the scope or puts it back to sleep.
    pub fn wake(&self) {
        let data = self.alive("waking");
        if data.lifetime.asleep.is_asleep() {
            observer::tell(*self, || Event::Woke { scope: self.id() });
        }
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
        self.data().is_some_and(|data| !data.lifetime.asleep.is_asleep())
    }

    /// Whether this scope is awake, for what it owns to ask later.
    pub fn awake(&self) -> Awake {
        match self.data() {
            Some(data) => Awake(data.lifetime.asleep.clone()),
            None => Awake(Arc::new(Gate::asleep())),
        }
    }

    /// Runs `hook` every time this scope wakes: for a feature to catch up on
    /// what it missed while it slept.
    pub fn on_wake(&self, hook: impl Fn() + 'static) {
        let data = self.alive("waiting for a wake");
        data.lifetime.wake_hooks.borrow_mut().push(Rc::new(hook));
    }

    /// Runs `hook` every time this scope is put to sleep: for what it owns to
    /// stop counting it as there.
    pub fn on_sleep(&self, hook: impl Fn() + 'static) {
        let data = self.alive("waiting for a sleep");
        data.lifetime.sleep_hooks.borrow_mut().push(Rc::new(hook));
    }
}
