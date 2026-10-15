use crate::actor::cancel::Cancel;
use crate::actor::ctx::task::Lanes;
use crate::actor::envelope::{self, Envelope, MessageEnvelope};
use crate::actor::event_bus::subscribe::{BusSubscription, Event};
use crate::actor::event_bus::EventBus;
use crate::actor::shape::name;
use crate::actor::flow::EventSubscription;
use crate::actor::traits::Handler;
use crate::actor::ManagedActor;
use crate::inspect::{Node, Open};
use crate::scope::{Scope, Section};
use crate::short_type_name;
use crate::trace::{self, Cause, Point};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};


/// Where an actor is made: the section of the scope that owns it, and the bus
/// of its tree.
///
/// What an actor makes lives where it does, so a feature's actor making
/// another one after the feature installed still makes it the feature's.
#[derive(Clone)]
pub struct Home {
    section: Section,
    bus: Weak<EventBus>,
}

impl Home {
    /// A home in `scope`'s own section, outside any feature.
    pub fn new(scope: Scope) -> Self {
        Self::in_section(scope.section())
    }

    /// A home in `section`.
    ///
    /// # Panics
    ///
    /// When its scope is already gone.
    pub fn in_section(section: Section) -> Self {
        let scope = section.scope();
        let bus = scope
            .tree_bus()
            .filter(|_| scope.is_alive())
            .unwrap_or_else(|| panic!("a home in {scope:?}, which was removed"));

        Self {
            section,
            bus: Rc::downgrade(&bus),
        }
    }

    /// A home in a new scope under this one, for something that comes and
    /// goes: removing that scope ends it, and it is still the same feature's.
    pub fn child(&self) -> Home {
        let scope = self.scope().child();
        Self::in_section(scope.open_section(self.section.feature(), self.section.declared()))
    }

    /// The scope that owns what is made here.
    pub fn scope(&self) -> Scope {
        self.section.scope()
    }

    /// The section what is made here belongs to.
    pub fn section(&self) -> Section {
        self.section
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

    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
}

pub struct Addr<A: 'static>(Rc<Actor<A>>);

struct Actor<A: 'static> {
    id: usize,
    state: RefCell<A>,
    queue: RefCell<VecDeque<Box<dyn Envelope<A>>>>,
    is_processing: Cell<bool>,
    cancel: Cancel,
    lanes: RefCell<Lanes>,
    /// What it hears, for as long as it lives: disposing it ends them.
    subscriptions: RefCell<Vec<BusSubscription>>,
    home: Home,
}

impl<A: 'static> Clone for Addr<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A: 'static> Addr<A> {
    /// An actor living in `home`, as `actor!` declared it: its scope owns it
    /// and disposes it when the scope goes, it hears what its manifest
    /// subscribes to until then, and devtools list it under the feature
    /// installing now.
    ///
    /// ```
    /// use guinea_core::actor::event_bus::Event;
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    ///
    /// #[derive(Clone)]
    /// struct Saved;
    ///
    /// impl Event for Saved {}
    ///
    /// #[derive(Debug)]
    /// struct Autosave;
    ///
    /// guinea_macros::actor! {
    ///     Autosave {
    ///         subscribes { Saved }
    ///     }
    /// }
    ///
    /// impl Handler<Saved> for Autosave {
    ///     fn handle(&mut self, _: Saved, _cx: Cx<Self, Saved>) {}
    /// }
    ///
    /// let tree = ScopeTree::new();
    /// let bus = tree.bus();
    /// let id = Addr::new(Autosave, &Home::new(tree.scope())).id();
    /// assert!(tree.actor(id).is_some(), "held by its scope");
    /// assert_eq!(bus.count_subscribers::<Saved>(), 1, "hearing its manifest");
    ///
    /// tree.remove();
    /// assert_eq!(bus.count_subscribers::<Saved>(), 0);
    /// ```
    ///
    /// # Panics
    ///
    /// When the scope of `home` is already gone.
    pub fn new(state: A, home: &Home) -> Self
    where
        A: ManagedActor,
    {
        Self::listed(state, home, None)
    }

    /// An actor with no declaration behind it: held by its scope, and nothing
    /// else - it hears nothing it did not subscribe to by hand, and devtools
    /// do not list it.
    pub(crate) fn bare(state: A, home: &Home) -> Self {
        let addr = Self(Rc::new(Actor {
            id: crate::scope::next_key(),
            state: RefCell::new(state),
            queue: RefCell::new(VecDeque::new()),
            is_processing: Cell::new(false),
            cancel: Cancel::new(),
            lanes: RefCell::default(),
            subscriptions: RefCell::new(Vec::new()),
            home: home.clone(),
        }));

        home.scope().hold(&addr);
        addr
    }

    /// Where it lives: what an actor it makes lives in too, when it is the
    /// actor's to make.
    pub fn home(&self) -> Home {
        self.0.home.clone()
    }

    /// Hears `M` on the bus of its tree for as long as the actor lives. What
    /// an actor hears is its manifest's `subscribes { .. }`, which calls this
    /// when the actor is made; nothing else does.
    pub(crate) fn subscribe<M: Event>(&self)
    where
        A: Handler<M>,
    {
        let bus = self.0.home.bus.upgrade().unwrap_or_else(|| {
            panic!(
                "{} lives in a tree that is gone, so there is no bus to hear {} on",
                short_type_name::<A>(),
                short_type_name::<M>()
            )
        });

        self.0.home.section.note_listener(name::<M>(), Some(name::<A>()));

        let subscription = bus.subscribe::<A, M>(self.clone());
        self.0.subscriptions.borrow_mut().push(subscription);
    }

    /// Hands `msg` to the actor. One that is not busy handles it before this
    /// returns:
    ///
    /// ```
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    ///
    /// #[derive(Debug)]
    /// struct Total(u32);
    ///
    /// struct Add(u32);
    ///
    /// guinea_macros::actor! {
    ///     Total {
    ///         handlers { Add }
    ///     }
    /// }
    ///
    /// impl Handler<Add> for Total {
    ///     fn handle(&mut self, Add(n): Add, _cx: Cx<Self, Add>) {
    ///         self.0 += n;
    ///     }
    /// }
    ///
    /// let tree = ScopeTree::new();
    /// let total = Addr::new(Total(0), &Home::new(tree.scope()));
    ///
    /// total.send(Add(2));
    /// total.send(Add(3));
    /// assert_eq!(total.inspect(None).preview, "Total(5)");
    /// ```
    ///
    /// One that is busy - sending to itself from a handler - handles it once
    /// what it is doing returns, in the order it was sent:
    ///
    /// ```
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    ///
    /// #[derive(Debug)]
    /// struct Countdown(Vec<u32>);
    ///
    /// struct From(u32);
    ///
    /// guinea_macros::actor! {
    ///     Countdown {
    ///         handlers { From }
    ///     }
    /// }
    ///
    /// impl Handler<From> for Countdown {
    ///     fn handle(&mut self, From(n): From, cx: Cx<Self, From>) {
    ///         if n > 0 {
    ///             cx.addr().send(From(n - 1));
    ///         }
    ///         self.0.push(n);
    ///     }
    /// }
    ///
    /// let tree = ScopeTree::new();
    /// let countdown = Addr::new(Countdown(Vec::new()), &Home::new(tree.scope()));
    ///
    /// countdown.send(From(2));
    /// assert_eq!(countdown.inspect(None).preview, "Countdown([2, 1, 0])");
    /// ```
    pub fn send<M>(&self, msg: M)
    where
        M: 'static,
        A: Handler<M>,
    {
        self.send_under(msg, trace::current());
    }

    pub fn id(&self) -> usize {
        self.0.id
    }

    /// The token every task this actor spawned is guarded by; cancelled when
    /// the actor's scope goes.
    pub fn cancellation(&self) -> Cancel {
        self.0.cancel.clone()
    }

    /// Its state, read as far as `open` reaches - the preview alone when
    /// `None`. Busy while it is handling a message.
    pub fn inspect(&self, open: Option<&Open>) -> Node
    where
        A: ManagedActor,
    {
        match self.0.state.try_borrow() {
            Ok(state) => Node::shown(|at| state.show(at), open),
            Err(_) => Node::busy("<handling a message>"),
        }
    }

    /// Ends its background work and what it hears: what it spawned is
    /// dropped where it last awaited, instead of running on with nowhere to
    /// answer. Its scope does this as it lets go of it.
    pub(crate) fn dispose(&self) {
        self.0.cancel.cancel();
        self.close_lanes();
        self.0.subscriptions.borrow_mut().clear();
    }

    /// Where `cx.task()` keeps what its policies hold back.
    pub(crate) fn lanes(&self) -> &RefCell<Lanes> {
        &self.0.lanes
    }
}

impl<A: 'static> Addr<A> {
    /// [`Addr::new`] for the actor that drives the reducer `drives`.
    pub(crate) fn listed(state: A, home: &Home, drives: Option<&'static str>) -> Self
    where
        A: ManagedActor,
    {
        let addr = Self::bare(state, home);
        A::Bus::subscribe_into(&addr);
        home.section.list(&addr, drives);
        addr
    }

    /// How work off the UI thread finds this actor again.
    pub(crate) fn reach(&self) -> Reach<A> {
        Reach {
            scope: self.0.home.scope(),
            id: self.0.id,
            actor: PhantomData,
        }
    }

    /// The bus of the tree it lives in, while the tree is there.
    pub(crate) fn tree_bus(&self) -> Option<Rc<EventBus>> {
        self.0.home.bus.upgrade()
    }

    /// Whether the scope it lives in is asleep, or gone: what a bus carries
    /// is not for it.
    pub(crate) fn is_asleep(&self) -> bool {
        !self.0.home.scope().is_awake()
    }

    /// Sends `msg` as caused by `parent`: for a message whose cause crossed a
    /// thread or a background task to get here. An actor that is not busy
    /// handles it at once; a busy one queues it for after what it is doing.
    pub(crate) fn send_under<M>(&self, msg: M, parent: Option<Cause>)
    where
        M: 'static,
        A: Handler<M>,
    {
        let cause = trace::mark_under(parent, || Point::Send {
            actor: short_type_name::<A>(),
            message: short_type_name::<M>(),
        });

        if self.0.is_processing.get() {
            self.0.queue.borrow_mut().push_back(Box::new(MessageEnvelope {
                message: Some(msg),
                cause,
            }));
            return;
        }
        self.0.is_processing.set(true);
        let _turn = Turn(&self.0);

        crate::notify::turn(|| {
            self.handled(|actor| envelope::deliver(actor, self, msg, cause));
            self.drain_queue();
        });
    }

    fn handled(&self, handle: impl FnOnce(&mut A)) {
        handle(&mut self.0.state.borrow_mut());

        crate::observability::changes::changed(|| {
            crate::observability::changes::Change::ActorHandled { id: self.0.id }
        });
    }

    /// Handles what was queued while the actor was busy, until the queue is
    /// empty or the actor is disposed.
    fn drain_queue(&self) {
        while !self.0.cancel.is_cancelled() {
            let next = self.0.queue.borrow_mut().pop_front();
            let Some(mut queued) = next else { break };

            self.handled(|actor| Envelope::<A>::handle(queued.as_mut(), actor, self));
        }
    }
}

/// An actor's turn at its messages: when it ends, however it ends, the actor
/// takes messages again, and what is still queued - behind a disposal or a
/// handler that panicked - is dropped unhandled.
struct Turn<'a, A: 'static>(&'a Actor<A>);

impl<A: 'static> Drop for Turn<'_, A> {
    fn drop(&mut self) {
        let left = std::mem::take(&mut *self.0.queue.borrow_mut());

        if std::thread::panicking() && !left.is_empty() {
            tracing::warn!(
                "{} panicked handling a message; the {} queued behind it are dropped",
                short_type_name::<A>(),
                left.len()
            );
        }

        drop(left);
        self.0.is_processing.set(false);
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
        let home = Home::new(tree.scope());
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
    fn an_address_is_one_pointer_so_handing_it_on_is_one_count() {
        assert_eq!(size_of::<Addr<()>>(), size_of::<usize>());
    }

    #[test]
    fn an_actor_whose_scope_was_removed_hears_nothing_more() {
        let home = TestHome::new();
        let addr = Addr::bare((), &home);

        home.tree.remove();

        assert!(addr.is_asleep(), "a removed scope read as awake");
    }

    #[test]
    fn an_actor_is_held_by_its_scope_alone() {
        let home = TestHome::new();
        let addr = Addr::bare((), &home);
        let held = Rc::downgrade(&addr.0);
        drop(addr);

        let while_there = held.strong_count();
        home.tree.remove();
        let after = held.strong_count();

        assert_eq!((while_there, after), (1, 0));
    }

    #[test]
    fn work_off_the_ui_thread_finds_its_actor_until_the_scope_goes() {
        let home = TestHome::new();
        let addr = Addr::bare((), &home);
        let reach = addr.reach();

        let while_there = reach.addr().map(|found| found.id());
        home.tree.remove();
        let after = reach.addr().map(|found| found.id());

        assert_eq!((while_there, after), (Some(addr.id()), None));
    }

    #[test]
    fn an_actor_is_not_found_as_another_type() {
        let home = TestHome::new();
        let addr = Addr::bare((), &home);

        let found = home.scope().held::<u8>(addr.id()).map(|found| found.id());

        assert_eq!(found, None);
    }

    #[test]
    fn making_an_actor_in_a_scope_that_is_gone_is_a_mistake_said_out_loud() {
        let home = TestHome::new();
        home.tree.remove();

        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Addr::bare((), &home);
        }));

        let message = made
            .err()
            .and_then(|panic| panic.downcast::<String>().ok())
            .map(|message| *message);
        let expected = format!("making an actor in {:?}, which was removed", home.scope());
        assert_eq!(message, Some(expected));
    }

    fn homeless(scope: Scope) -> Option<String> {
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Home::new(scope);
        }));

        made.err()
            .and_then(|panic| panic.downcast::<String>().ok())
            .map(|message| *message)
    }

    #[test]
    fn a_home_in_a_removed_scope_is_a_mistake_said_out_loud() {
        let tree = crate::scope::ScopeTree::new();
        let gone = tree.child();
        gone.remove();

        assert_eq!(homeless(gone), Some(format!("a home in {gone:?}, which was removed")));
    }

    #[test]
    fn a_home_in_a_tree_that_is_gone_is_a_mistake_said_out_loud() {
        let tree = crate::scope::ScopeTree::new();
        let gone = tree.scope();
        drop(tree);

        assert_eq!(homeless(gone), Some(format!("a home in {gone:?}, which was removed")));
    }

    #[derive(Clone)]
    struct Heard;

    impl Event for Heard {}

    #[derive(Debug)]
    struct Listening;

    guinea_macros::actor! {
        Listening {
            subscribes { Heard }
        }
    }

    impl Handler<Heard> for Listening {
        fn handle(&mut self, _: Heard, _cx: crate::actor::Cx<Self, Heard>) {}
    }

    struct Counting(Rc<Cell<u32>>);

    impl Handler<Heard> for Counting {
        fn handle(&mut self, _: Heard, _cx: crate::actor::Cx<Self, Heard>) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn an_actor_below_the_root_hears_the_bus_of_its_tree() {
        let tree = crate::scope::ScopeTree::new();
        let window = tree.child();
        let heard = Rc::new(Cell::new(0));
        let addr = Addr::bare(Counting(heard.clone()), &Home::new(window.child()));

        addr.subscribe::<Heard>();
        tree.bus().publish(Heard);

        assert_eq!(heard.get(), 1);
    }

    struct Leave;
    struct After;

    struct Leaving {
        scope: Scope,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Handler<Leave> for Leaving {
        fn handle(&mut self, _: Leave, cx: crate::actor::Cx<Self, Leave>) {
            cx.addr().send(After);
            self.scope.remove();
            self.log.borrow_mut().push("leave");
        }
    }

    impl Handler<After> for Leaving {
        fn handle(&mut self, _: After, _cx: crate::actor::Cx<Self, After>) {
            self.log.borrow_mut().push("after");
        }
    }

    #[test]
    fn what_was_queued_for_an_actor_is_not_handled_once_its_scope_is_gone() {
        let tree = crate::scope::ScopeTree::new();
        let scope = tree.child();
        let log = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::bare(Leaving { scope, log: log.clone() }, &Home::new(scope));

        addr.send(Leave);

        assert_eq!(*log.borrow(), ["leave"], "handled after its scope was removed");
    }

    #[test]
    fn an_actor_whose_scope_goes_while_an_event_is_told_does_not_hear_it() {
        let tree = crate::scope::ScopeTree::new();
        let scope = tree.child();
        let heard = Rc::new(Cell::new(0));

        let _remover = tree.bus().subscribe_fn(move |_: Heard| scope.remove());
        let addr = Addr::bare(Counting(heard.clone()), &Home::new(scope));
        addr.subscribe::<Heard>();

        tree.bus().publish(Heard);

        assert_eq!(heard.get(), 0, "heard after its scope was removed");
    }

    struct Boom;

    impl Handler<Boom> for Counting {
        fn handle(&mut self, _: Boom, _cx: crate::actor::Cx<Self, Boom>) {
            panic!("boom");
        }
    }

    #[test]
    fn an_actor_whose_handler_panicked_still_handles_what_comes_next() {
        let home = TestHome::new();
        let heard = Rc::new(Cell::new(0));
        let addr = Addr::bare(Counting(heard.clone()), &home);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| addr.send(Boom)));
        addr.send(Heard);

        assert!(panicked.is_err());
        assert_eq!(heard.get(), 1, "the message after the panic was never handled");
    }

    struct Fuse;

    impl Handler<Fuse> for Counting {
        fn handle(&mut self, _: Fuse, cx: crate::actor::Cx<Self, Fuse>) {
            cx.addr().send(Heard);
            panic!("fuse");
        }
    }

    #[test]
    fn what_a_panicking_handler_queued_goes_with_it() {
        let home = TestHome::new();
        let heard = Rc::new(Cell::new(0));
        let addr = Addr::bare(Counting(heard.clone()), &home);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| addr.send(Fuse)));
        addr.send(Heard);

        assert!(panicked.is_err());
        assert_eq!(heard.get(), 1, "what the panicking turn queued was handled after all");
    }

    #[test]
    fn a_managed_actor_hears_its_manifest_until_its_scope_goes() {
        let home = TestHome::new();
        let bus = home.tree.bus();
        let _addr = Addr::new(Listening, &home);
        assert_eq!(bus.count_subscribers::<Heard>(), 1);

        home.tree.remove();

        assert_eq!(bus.count_subscribers::<Heard>(), 0);
    }
}
