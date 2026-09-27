use crate::actor::cancel::Cancel;
use crate::actor::envelope::{Envelope, FnEnvelope, MessageEnvelope};
use crate::actor::event_bus::builder::EventSubscription;
use crate::actor::event_bus::subscribe::{BusSubscription, Event};
use crate::actor::event_bus::{EventBus, GlobalEventBus};
use crate::actor::shape::name;
use crate::actor::traits::Handler;
use crate::actor::{Cx, UiThreadToken};
use crate::actor::{ManagedActor, short_type_name};
use crate::lifecycle_tracker::LifecycleTracker;
use crate::scope::Scope;
use crate::trace::{self, Bus, Cause, Point};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    pub static REGISTRY: RefCell<HashMap<usize, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

pub struct Addr<A: 'static> {
    pub(super) id: usize,
    pub(super) guard: UiThreadToken,
    state: Rc<RefCell<A>>,
    queue: Rc<RefCell<VecDeque<Box<dyn Envelope<A>>>>>,
    is_processing: Rc<Cell<bool>>,
    counter: Rc<&'static str>,
    cancel: Cancel,
    /// What it hears, for as long as it lives: disposing it ends them.
    subscriptions: Rc<RefCell<Vec<BusSubscription>>>,
    /// The scope it belongs to and its window's bus, when it has them.
    home: Rc<RefCell<Option<Home>>>,
}

struct Home {
    scope: Weak<Scope>,
    bus: Weak<EventBus>,
}

impl<A: 'static> Clone for Addr<A> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            state: self.state.clone(),
            guard: self.guard.clone(),
            queue: self.queue.clone(),
            is_processing: self.is_processing.clone(),
            counter: self.counter.clone(),
            cancel: self.cancel.clone(),
            subscriptions: self.subscriptions.clone(),
            home: self.home.clone(),
        }
    }
}

/// Keeps what an actor subscribed to on the actor itself.
struct Held(Rc<RefCell<Vec<BusSubscription>>>);

impl LifecycleTracker for Held {
    fn track_loop<T: 'static>(&self, _handle: T) {}
    fn track_actor<A: 'static>(&self, _addr: &Addr<A>) {}

    fn track_sub(&self, subscription: BusSubscription) {
        self.0.borrow_mut().push(subscription);
    }
}

impl<A: 'static> Addr<A> {
    pub fn new_managed(state: A, token: UiThreadToken, tracker: &impl LifecycleTracker) -> Self
    where
        A: ManagedActor,
    {
        let addr = Self::new(state, token, tracker);

        A::Bus::subscribe_into(addr.clone(), tracker);

        addr
    }

    /// An actor a scope owns. What its manifest subscribes to is held by the
    /// actor, and ends when the scope disposes it.
    pub fn new_managed_scoped(state: A, token: UiThreadToken) -> Self
    where
        A: ManagedActor,
    {
        let addr = Self::new(state, token, &crate::lifecycle_tracker::NullTracker);
        A::Bus::subscribe_into(addr.clone(), &Held(addr.subscriptions.clone()));
        addr
    }

    /// Where the actor lives: the scope that owns it, and that scope's
    /// window bus. What `subscribe_on` reaches the window bus through, and
    /// notes a listener on.
    #[doc(hidden)]
    pub fn live_in(&self, scope: &Rc<Scope>, bus: &Rc<EventBus>) {
        *self.home.borrow_mut() = Some(Home {
            scope: Rc::downgrade(scope),
            bus: Rc::downgrade(bus),
        });
    }

    /// Hears `M` on `bus` for as long as the actor lives: disposing it ends
    /// the subscription.
    pub fn subscribe_on<M: Event>(&self, bus: Bus)
    where
        A: Handler<M>,
    {
        let (scope, window) = match self.home.borrow().as_ref() {
            Some(home) => (home.scope.upgrade(), home.bus.upgrade()),
            None => (None, None),
        };

        let on = match bus {
            Bus::Global => GlobalEventBus::bus(),
            Bus::Window => window.unwrap_or_else(|| {
                panic!(
                    "{} lives in no window, so there is no window bus to hear {} on",
                    short_type_name::<A>(),
                    short_type_name::<M>()
                )
            }),
        };

        if let Some(scope) = scope {
            scope.note_listener(name::<M>(), Some(name::<A>()), bus);
        }

        let subscription = on.subscribe::<A, M>(self.clone());
        self.subscriptions.borrow_mut().push(subscription);
    }

    pub fn new_scoped(state: A, token: UiThreadToken) -> Self {
        Self::new(state, token, &crate::lifecycle_tracker::NullTracker)
    }

    pub fn new(state: A, guard: UiThreadToken, tracker: &impl LifecycleTracker) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let addr = Self {
            id,
            guard,
            state: Rc::new(RefCell::new(state)),
            queue: Rc::new(RefCell::new(VecDeque::new())),
            is_processing: Rc::new(Cell::new(false)),
            counter: Rc::new(short_type_name::<A>()),
            cancel: Cancel::new(),
            subscriptions: Rc::new(RefCell::new(Vec::new())),
            home: Rc::new(RefCell::new(None)),
        };

        let addr_clone = addr.clone();
        REGISTRY.with(|reg| {
            reg.borrow_mut().insert(id, Box::new(addr_clone));
        });

        tracker.track_actor(&addr);
        addr
    }

    pub fn apply<F>(&self, f: F)
    where
        F: FnOnce(&mut A, &Cx<A>) + Send + 'static,
    {
        self.queue.borrow_mut().push_back(Box::new(FnEnvelope {
            func: Some(f),
            cause: trace::current(),
            phantom: PhantomData,
        }));

        self.process_queue();
    }

    pub fn handler<M>(&self, msg: M) -> impl Fn() + 'static
    where
        M: Clone + 'static,
        A: Handler<M>,
    {
        let addr = self.clone();
        move || addr.do_send(msg.clone())
    }

    pub fn handler_with<M, T, F>(&self, f: F) -> impl Fn(T) + 'static
    where
        F: Fn(T) -> M + 'static,
        M: 'static,
        A: Handler<M>,
    {
        let addr = self.clone();
        move |arg| addr.do_send(f(arg))
    }

    pub fn handler_with2<M, T1, T2, F>(&self, f: F) -> impl Fn(T1, T2) + 'static
    where
        F: Fn(T1, T2) -> M + 'static,
        M: 'static,
        A: Handler<M>,
    {
        let addr = self.clone();
        move |arg1, arg2| addr.do_send(f(arg1, arg2))
    }

    pub fn send<M>(&self, msg: M)
    where
        M: 'static,
        A: Handler<M>,
    {
        self.do_send(msg);
    }

    #[cfg(feature = "test-utils")]
    pub fn send_test<M>(&self, msg: M) -> crate::test_kit::Interaction<()>
    where
        M: 'static,
        A: Handler<M>,
    {
        self.do_send(msg);
        crate::test_kit::Interaction::new(())
    }

    fn do_send<M>(&self, msg: M)
    where
        M: 'static,
        A: Handler<M>,
    {
        self.send_under(msg, trace::current());
    }

    /// Queues `msg` as caused by `parent`: for a message whose cause crossed a
    /// thread or a background task to get here.
    pub(crate) fn send_under<M>(&self, msg: M, parent: Option<Cause>)
    where
        M: 'static,
        A: Handler<M>,
    {
        let cause = trace::mark_under(parent, || Point::Send {
            actor: short_type_name::<A>(),
            message: short_type_name::<M>(),
        });

        self.queue.borrow_mut().push_back(Box::new(MessageEnvelope {
            message: Some(msg),
            cause,
        }));

        self.process_queue();
    }

    pub fn get_token(&self) -> UiThreadToken {
        self.guard.clone()
    }
    pub fn strong_count_ptr(&self) -> Rc<&'static str> {
        self.counter.clone()
    }

    pub fn id(&self) -> usize {
        self.id
    }

    /// The token every task this actor spawned is guarded by; cancelled by
    /// [`Addr::dispose`], and so by the teardown that owns the actor.
    pub fn cancellation(&self) -> Cancel {
        self.cancel.clone()
    }

    pub fn debug_snapshot(&self) -> String
    where
        A: std::fmt::Debug,
    {
        match self.state.try_borrow() {
            Ok(state) => format!("{:#?}", *state),
            Err(_) => "<handling a message>".to_string(),
        }
    }

    /// Takes the actor out of the registry and ends its background work: what
    /// it spawned is dropped where it last awaited, instead of running on with
    /// nowhere to answer.
    pub fn dispose(&self) {
        self.cancel.cancel();
        self.subscriptions.borrow_mut().clear();

        REGISTRY.with(|reg| {
            reg.borrow_mut().remove(&self.id);
        });
    }

    fn process_queue(&self) {
        if self.is_processing.get() {
            return;
        }
        self.is_processing.set(true);

        crate::notify::turn(|| self.drain_queue());
    }

    fn drain_queue(&self) {
        loop {
            let mut envelope = {
                let mut q = self.queue.borrow_mut();
                match q.pop_front() {
                    Some(e) => e,
                    None => {
                        self.is_processing.set(false);
                        break;
                    }
                }
            };

            {
                let mut state_guard = self.state.borrow_mut();
                Envelope::<A>::handle(envelope.as_mut(), &mut *state_guard, self);
            }
        }

        self.is_processing.set(false);
    }
}
