use crate::actor::cancel::Cancel;
use crate::actor::envelope::{Envelope, MessageEnvelope};
use crate::actor::event_bus::subscribe::{BusSubscription, Event};
use crate::actor::event_bus::{EventBus, GlobalEventBus};
use crate::actor::shape::name;
use crate::actor::traits::{EventSubscription, Handler};
use crate::actor::UiThreadToken;
use crate::actor::{ManagedActor, short_type_name};
use crate::scope::Scope;
use crate::trace::{self, Bus, Cause, Point};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    pub static REGISTRY: RefCell<HashMap<usize, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// The actor registered under `id`, if it is still there and is an `A`.
///
/// A clone, taken with the registry borrowed only for as long as it takes to
/// clone: whatever the caller does with it next - send, and so run handlers
/// that create or dispose actors - finds the registry free.
pub(crate) fn registered<A: 'static>(id: usize) -> Option<Addr<A>> {
    REGISTRY.with(|reg| {
        reg.borrow()
            .get(&id)
            .and_then(|addr| addr.downcast_ref::<Addr<A>>())
            .cloned()
    })
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
    scope: Scope,
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

impl<A: 'static> Addr<A> {
    /// An actor `actor!` declared. What its manifest subscribes to is held by
    /// the actor, and ends when it is disposed.
    pub fn new_managed(state: A, token: UiThreadToken) -> Self
    where
        A: ManagedActor,
    {
        let addr = Self::new(state, token);
        A::Bus::subscribe_into(&addr);
        addr
    }

    /// Where the actor lives: the scope that owns it, and that scope's
    /// window bus when it is in a window. What `subscribe_on` reaches the
    /// window bus through, and notes a listener on.
    #[doc(hidden)]
    pub fn live_in(&self, scope: Scope, bus: Option<&Rc<EventBus>>) {
        *self.home.borrow_mut() = Some(Home {
            scope,
            bus: bus.map(Rc::downgrade).unwrap_or_default(),
        });
    }

    /// Hears `M` on `bus` for as long as the actor lives: disposing it ends
    /// the subscription.
    pub fn subscribe_on<M: Event>(&self, bus: Bus)
    where
        A: Handler<M>,
    {
        let (scope, window) = match self.home.borrow().as_ref() {
            Some(home) => (Some(home.scope), home.bus.upgrade()),
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

    /// Whether the scope it lives in is asleep, or gone: what a bus carries
    /// is not for it.
    pub(crate) fn is_asleep(&self) -> bool {
        self.home
            .borrow()
            .as_ref()
            .is_some_and(|home| !home.scope.is_awake())
    }

    pub fn new(state: A, guard: UiThreadToken) -> Self {
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

        addr
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

        let gone = REGISTRY.with(|reg| reg.borrow_mut().remove(&self.id));
        drop(gone);
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
            crate::devtools::changed(|| crate::devtools::Change::ActorHandled { id: self.id });
        }

        self.is_processing.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_actor_whose_scope_was_removed_hears_nothing_more() {
        let scope = crate::scope::ScopeTree::new();
        let addr = Addr::new((), UiThreadToken::dangerously_create_token_unchecked());
        addr.live_in(scope.scope(), Some(&Rc::new(EventBus::new())));

        scope.remove();

        assert!(addr.is_asleep(), "a removed scope read as awake");
    }

    #[derive(Clone)]
    struct Heard;

    impl Event for Heard {}

    #[derive(Debug)]
    struct Listening;

    guinea_macros::actor! {
        Listening {
            handlers { Heard }
            subscribes { Heard }
        }
    }

    impl Handler<Heard> for Listening {
        fn handle(&mut self, _: Heard, _cx: crate::actor::Cx<Self, Heard>) {}
    }

    #[test]
    fn a_managed_actor_hears_its_manifest_until_it_is_disposed() {
        let addr = Addr::new_managed(Listening, UiThreadToken::dangerously_create_token_unchecked());
        assert_eq!(GlobalEventBus::count_subscribers::<Heard>(), 1);

        addr.dispose();

        assert_eq!(GlobalEventBus::count_subscribers::<Heard>(), 0);
    }
}
