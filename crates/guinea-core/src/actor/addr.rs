use crate::actor::cancel::Cancel;
use crate::actor::envelope::{Envelope, MessageEnvelope};
use crate::actor::event_bus::subscribe::{BusSubscription, Event};
use crate::actor::event_bus::{EventBus, GlobalEventBus};
use crate::actor::shape::name;
use crate::actor::traits::{EventSubscription, Handler};
use crate::actor::{ManagedActor, short_type_name};
use crate::scope::Scope;
use crate::trace::{self, Bus, Cause, Point};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// Where an actor is made: the scope that owns it, and the window bus it can
/// hear when it is in a window.
#[derive(Clone)]
pub struct Home {
    scope: Scope,
    bus: Weak<EventBus>,
}

impl Home {
    pub fn new(scope: Scope, bus: Option<&Rc<EventBus>>) -> Self {
        Self {
            scope,
            bus: bus.map(Rc::downgrade).unwrap_or_default(),
        }
    }

    /// The scope that owns what is made here.
    pub fn scope(&self) -> Scope {
        self.scope
    }
}

/// How work off the UI thread finds its actor again: by the scope that owns
/// it and its id there. `Send`, where an [`Addr`] is not.
pub(crate) struct Reach<A> {
    scope: Scope,
    id: usize,
    actor: PhantomData<fn() -> A>,
}

impl<A> Clone for Reach<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for Reach<A> {}

impl<A: 'static> Reach<A> {
    /// The actor, while its scope still holds it. On the UI thread.
    pub(crate) fn addr(&self) -> Option<Addr<A>> {
        self.scope.held::<A>(self.id)
    }

    pub(crate) fn id(&self) -> usize {
        self.id
    }
}

pub struct Addr<A: 'static> {
    pub(super) id: usize,
    state: Rc<RefCell<A>>,
    queue: Rc<RefCell<VecDeque<Box<dyn Envelope<A>>>>>,
    is_processing: Rc<Cell<bool>>,
    counter: Rc<&'static str>,
    cancel: Cancel,
    /// What it hears, for as long as it lives: disposing it ends them.
    subscriptions: Rc<RefCell<Vec<BusSubscription>>>,
    home: Rc<Home>,
}

impl<A: 'static> Clone for Addr<A> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            state: self.state.clone(),
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
    /// An actor living in `home`: its scope owns it, and disposes it when
    /// the scope goes. Made in a scope that is already gone, it is disposed
    /// at once.
    pub fn new(state: A, home: &Home) -> Self {
        let addr = Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            state: Rc::new(RefCell::new(state)),
            queue: Rc::new(RefCell::new(VecDeque::new())),
            is_processing: Rc::new(Cell::new(false)),
            counter: Rc::new(short_type_name::<A>()),
            cancel: Cancel::new(),
            subscriptions: Rc::new(RefCell::new(Vec::new())),
            home: Rc::new(home.clone()),
        };

        home.scope.hold(&addr);
        addr
    }

    /// An actor `actor!` declared: subscribed to its manifest until it is
    /// disposed, and listed for devtools under the feature installing now.
    pub fn new_managed(state: A, home: &Home) -> Self
    where
        A: ManagedActor + Debug,
    {
        Self::listed(state, home, None)
    }

    /// [`Addr::new_managed`] for the actor that drives the reducer `drives`.
    pub(crate) fn listed(state: A, home: &Home, drives: Option<&'static str>) -> Self
    where
        A: ManagedActor + Debug,
    {
        let addr = Self::new(state, home);
        A::Bus::subscribe_into(&addr);
        home.scope.list(&addr, drives);
        addr
    }

    /// Where it lives: what an actor it makes lives in too, when it is the
    /// actor's to make.
    pub fn home(&self) -> Home {
        (*self.home).clone()
    }

    /// How work off the UI thread finds this actor again.
    pub(crate) fn reach(&self) -> Reach<A> {
        Reach {
            scope: self.home.scope,
            id: self.id,
            actor: PhantomData,
        }
    }

    /// Hears `M` on `bus` for as long as the actor lives: disposing it ends
    /// the subscription.
    pub fn subscribe_on<M: Event>(&self, bus: Bus)
    where
        A: Handler<M>,
    {
        let on = match bus {
            Bus::Global => GlobalEventBus::bus(),
            Bus::Window => self.home.bus.upgrade().unwrap_or_else(|| {
                panic!(
                    "{} lives in no window, so there is no window bus to hear {} on",
                    short_type_name::<A>(),
                    short_type_name::<M>()
                )
            }),
        };

        self.home.scope.note_listener(name::<M>(), Some(name::<A>()), bus);

        let subscription = on.subscribe::<A, M>(self.clone());
        self.subscriptions.borrow_mut().push(subscription);
    }

    /// Whether the scope it lives in is asleep, or gone: what a bus carries
    /// is not for it.
    pub(crate) fn is_asleep(&self) -> bool {
        !self.home.scope.is_awake()
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

    /// Ends its background work and what it hears: what it spawned is
    /// dropped where it last awaited, instead of running on with nowhere to
    /// answer. Its scope lets go of it, and does this itself when it goes.
    pub fn dispose(&self) {
        self.cancel.cancel();
        self.subscriptions.borrow_mut().clear();
        self.home.scope.release(self.id);
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
            crate::observability::changes::changed(|| {
                crate::observability::changes::Change::ActorHandled { id: self.id }
            });
        }

        self.is_processing.set(false);
    }
}

/// A scope tree of its own for a unit test, and the home in it: actors made
/// there live until it is dropped.
#[cfg(test)]
pub(crate) struct TestHome {
    pub(crate) tree: crate::scope::ScopeTree,
    home: Home,
}

#[cfg(test)]
impl TestHome {
    pub(crate) fn new() -> Self {
        let tree = crate::scope::ScopeTree::new();
        let home = Home::new(tree.scope(), None);
        Self { tree, home }
    }
}

#[cfg(test)]
impl std::ops::Deref for TestHome {
    type Target = Home;

    fn deref(&self) -> &Home {
        &self.home
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_actor_whose_scope_was_removed_hears_nothing_more() {
        let home = TestHome::new();
        let addr = Addr::new((), &home);

        home.tree.remove();

        assert!(addr.is_asleep(), "a removed scope read as awake");
    }

    #[test]
    fn an_actor_is_held_by_its_scope_alone() {
        let home = TestHome::new();
        let addr = Addr::new((), &home);
        let counter = addr.strong_count_ptr();
        drop(addr);

        let while_there = Rc::strong_count(&counter);
        home.tree.remove();
        let after = Rc::strong_count(&counter);

        assert_eq!((while_there, after), (2, 1));
    }

    #[test]
    fn work_off_the_ui_thread_finds_its_actor_until_the_scope_goes() {
        let home = TestHome::new();
        let addr = Addr::new((), &home);
        let reach = addr.reach();

        let while_there = reach.addr().map(|found| found.id());
        home.tree.remove();
        let after = reach.addr().map(|found| found.id());

        assert_eq!((while_there, after), (Some(addr.id()), None));
    }

    #[test]
    fn an_actor_disposed_before_its_scope_goes_is_let_go_at_once() {
        let home = TestHome::new();
        let addr = Addr::new((), &home);
        let counter = addr.strong_count_ptr();
        let reach = addr.reach();

        addr.dispose();
        drop(addr);

        let found = reach.addr().map(|found| found.id());
        assert_eq!((Rc::strong_count(&counter), found), (1, None));
    }

    #[test]
    fn an_actor_is_not_found_as_another_type() {
        let home = TestHome::new();
        let addr = Addr::new((), &home);

        let found = home.scope().held::<u8>(addr.id()).map(|found| found.id());

        assert_eq!(found, None);
    }

    #[test]
    fn an_actor_made_in_a_scope_that_is_gone_is_disposed_at_once() {
        let home = TestHome::new();
        home.tree.remove();

        let addr = Addr::new((), &home);

        assert!(addr.cancellation().is_cancelled());
        assert_eq!(addr.reach().addr().map(|found| found.id()), None);
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
        let home = TestHome::new();
        let addr = Addr::new_managed(Listening, &home);
        assert_eq!(GlobalEventBus::count_subscribers::<Heard>(), 1);

        addr.dispose();

        assert_eq!(GlobalEventBus::count_subscribers::<Heard>(), 0);
    }
}
