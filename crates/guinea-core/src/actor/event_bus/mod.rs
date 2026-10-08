use crate::actor::addr::Addr;
use crate::actor::event_bus::subscribe::{
    AnswerFn, BusSubscription, FnSubscriber, Subscriber, SubscriptionId, UntypedSubscriber,
};
use crate::actor::invoke_on_ui;
use crate::actor::short_type_name;
use crate::actor::traits::Handler;
use crate::observability::changes::{self, Change};
use crate::trace::{self, Bus, Point};
use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub mod rpc;
pub mod subscribe;
pub use rpc::{AsyncBus, Reply, RpcCall, RpcRequest, RpcResponse};
pub use subscribe::Event;

#[cfg(feature = "test-utils")]
pub static TEST_TASK_QUEUE: std::sync::LazyLock<std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

#[cfg(feature = "test-utils")]
pub static ACTIVE_TASKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One background task, counted in [`ACTIVE_TASKS`] until this is dropped -
/// whether the task answered or was cancelled.
#[cfg(feature = "test-utils")]
pub struct Counted;

#[cfg(feature = "test-utils")]
impl Counted {
    pub fn new() -> Self {
        ACTIVE_TASKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(feature = "test-utils")]
impl Default for Counted {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "test-utils")]
impl Drop for Counted {
    fn drop(&mut self) {
        ACTIVE_TASKS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// One event type on a bus and who hears it. See [`EventBus::listeners`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listening {
    pub event: &'static str,
    pub listeners: Vec<Listener>,
}

/// One subscriber of one event type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    pub by: HeardBy,
    /// It answers the request rather than only hearing it.
    pub answers: bool,
    /// It lives in a scope that is asleep, and hears nothing now.
    pub asleep: bool,
}

/// Who a subscriber is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeardBy {
    /// An actor, by its type and its id in the actor snapshot.
    Actor { name: &'static str, id: usize },
    /// A callback, by where it was subscribed.
    Callback(&'static std::panic::Location<'static>),
    /// A subscriber that does not say.
    Unknown,
}

pub struct EventBus {
    /// `Rc` rather than `Box`: delivery hands the subscribers out of the map
    /// before telling them, so one that subscribes or unsubscribes while it
    /// is being told does not find the map borrowed.
    subscribers: RefCell<HashMap<TypeId, Vec<Rc<dyn UntypedSubscriber>>>>,
    counts: RefCell<HashMap<TypeId, usize>>,
    /// The one subscriber that answers each request type, by its `seq`.
    answerers: RefCell<HashMap<TypeId, (u64, &'static str)>>,
    next_id: Cell<u64>,
    kind: Bus,
    /// The window it is the bus of, for [`Change`]; `None` for the global
    /// one, and for one no window owns.
    root: Option<u64>,
}

/// Whether a request published now would be answered.
pub(crate) enum Answering {
    Awake,
    Nobody,
    /// The answerer lives in a scope that is asleep.
    Asleep(&'static str),
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let subscribers: usize = self
            .subscribers
            .try_borrow()
            .map_or(0, |subscribers| subscribers.values().map(Vec::len).sum());
        f.debug_struct("EventBus")
            .field("kind", &self.kind)
            .field("subscribers", &subscribers)
            .finish()
    }
}

impl EventBus {
    /// A window's own bus.
    pub fn new() -> Self {
        Self::of_kind(Bus::Window)
    }

    /// The bus of window `root`, as `RootId::get` numbers it.
    pub fn for_root(root: u64) -> Self {
        Self {
            root: Some(root),
            ..Self::of_kind(Bus::Window)
        }
    }

    fn of_kind(kind: Bus) -> Self {
        Self {
            subscribers: RefCell::new(HashMap::new()),
            counts: RefCell::new(HashMap::new()),
            answerers: RefCell::new(HashMap::new()),
            next_id: Cell::new(0),
            kind,
            root: None,
        }
    }

    fn changed(&self) {
        changes::changed(|| Change::Subscriptions {
            bus: self.kind,
            root: self.root,
        });
    }

    pub fn kind(&self) -> Bus {
        self.kind
    }

    /// The event types something is subscribed to, with how many subscribers
    /// each has.
    pub fn subscriptions(&self) -> Vec<(&'static str, usize)> {
        let subscribers = self.subscribers.borrow();
        let mut listed: Vec<(&'static str, usize)> = subscribers
            .values()
            .filter(|list| !list.is_empty())
            .map(|list| (list[0].event(), list.len()))
            .collect();
        listed.sort_unstable();
        listed
    }

    /// Every event type something is subscribed to, with who hears it, in the
    /// order they subscribed.
    pub fn listeners(&self) -> Vec<Listening> {
        let answerers = self.answerers.borrow();
        let subscribers = self.subscribers.borrow();

        let mut listed: Vec<Listening> = subscribers
            .iter()
            .filter(|(_, list)| !list.is_empty())
            .map(|(event, list)| Listening {
                event: list[0].event(),
                listeners: list
                    .iter()
                    .map(|sub| Listener {
                        by: sub.heard_by(),
                        answers: answerers.get(event).is_some_and(|(seq, _)| *seq == sub.seq()),
                        asleep: sub.is_asleep(),
                    })
                    .collect(),
            })
            .collect();

        listed.sort_unstable_by_key(|listening| listening.event);
        listed
    }

    fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    pub fn subscribe<A, M>(self: &Rc<Self>, addr: Addr<A>) -> BusSubscription
    where
        A: Handler<M> + 'static,
        M: Event,
    {
        let seq = self.next_id();
        self.insert::<M>(Box::new(Subscriber {
            seq,
            addr,
            _marker: std::marker::PhantomData,
        }))
    }

    #[track_caller]
    pub fn subscribe_fn<M: Event>(
        self: &Rc<Self>,
        callback: impl Fn(M) + 'static,
    ) -> BusSubscription {
        let seq = self.next_id();
        self.insert::<M>(Box::new(FnSubscriber {
            seq,
            callback: Arc::new(callback),
            at: std::panic::Location::caller(),
        }))
    }

    /// Answers `Req` with `answer`: the callback counterpart of an actor's
    /// handler that returns the reply.
    #[track_caller]
    pub fn answer_fn<Req: RpcCall>(
        self: &Rc<Self>,
        answer: impl Fn(Req) -> Req::Response + 'static,
    ) -> BusSubscription {
        let seq = self.next_id();
        self.insert::<RpcRequest<Req>>(Box::new(AnswerFn {
            seq,
            answer: Box::new(answer),
            at: std::panic::Location::caller(),
        }))
    }

    fn insert<M: Event>(self: &Rc<Self>, subscriber: Box<dyn UntypedSubscriber>) -> BusSubscription {
        let event = TypeId::of::<M>();
        let id = SubscriptionId {
            seq: subscriber.seq(),
            event,
        };

        if let Some(answerer) = subscriber.answerer() {
            let mut answerers = self.answerers.borrow_mut();
            if let Some((_, already)) = answerers.get(&event) {
                panic!(
                    "{answerer} answers {} on the {:?} bus, and {already} already does: a request \
                     has exactly one answerer. Make one of them only hear it - a handler that \
                     returns nothing - or answer from one place.",
                    subscriber.event(),
                    self.kind,
                );
            }
            answerers.insert(event, (id.seq, answerer));
        }

        *self.counts.borrow_mut().entry(event).or_insert(0) += 1;
        self.subscribers
            .borrow_mut()
            .entry(event)
            .or_default()
            .push(Rc::from(subscriber));
        self.changed();

        BusSubscription {
            bus: Rc::downgrade(self),
            id,
        }
    }

    pub fn count_subscribers<M: Event>(&self) -> usize {
        let type_id = TypeId::of::<M>();
        *self.counts.borrow().get(&type_id).unwrap_or(&0)
    }

    pub fn has_subscribers<M: Event>(&self) -> bool {
        self.count_subscribers::<M>() > 0
    }

    pub fn publish<M: Event>(&self, msg: M) {
        let _published = trace::enter(|| Point::Publish {
            event: short_type_name::<M>(),
            bus: self.kind,
            subscribers: self.count_subscribers::<M>(),
        });

        // Told from a copy of the list, not from the map: a handler is free
        // to subscribe or unsubscribe while it is being told, and either
        // would borrow the map this delivery is walking. One that goes
        // mid-publication still hears this one out.
        let type_id = TypeId::of::<M>();
        let telling: Vec<Rc<dyn UntypedSubscriber>> = self
            .subscribers
            .borrow()
            .get(&type_id)
            .cloned()
            .unwrap_or_default();

        for sub in telling {
            // Around each one, not around the lot: the copy every subscriber
            // is handed is made here, and time spent making it belongs to the
            // delivery that needed it rather than to the publication as a
            // whole. Without this it is the difference between a publication
            // and the sum of its handlers, which is to say invisible.
            let _delivered = trace::enter(|| Point::Deliver {
                event: short_type_name::<M>(),
                bus: self.kind,
            });

            sub.deliver(Box::new(msg.clone()), self.kind);
        }
    }

    /// Whether a request `M` published now would be answered, and if not, why.
    pub(crate) fn answering<M: Event>(&self) -> Answering {
        let event = TypeId::of::<M>();
        let Some((seq, answerer)) = self.answerers.borrow().get(&event).copied() else {
            return Answering::Nobody;
        };

        let asleep = self
            .subscribers
            .borrow()
            .get(&event)
            .and_then(|list| list.iter().find(|sub| sub.seq() == seq).cloned())
            .is_some_and(|sub| sub.is_asleep());

        match asleep {
            true => Answering::Asleep(answerer),
            false => Answering::Awake,
        }
    }

    pub(super) fn remove(&self, id: SubscriptionId) {
        {
            let mut answerers = self.answerers.borrow_mut();
            if answerers.get(&id.event).is_some_and(|(seq, _)| *seq == id.seq) {
                answerers.remove(&id.event);
            }
        }

        let removed = {
            let mut subscribers = self.subscribers.borrow_mut();
            let Some(list) = subscribers.get_mut(&id.event) else {
                return;
            };

            let before = list.len();
            list.retain(|sub| sub.seq() != id.seq);
            before - list.len()
        };

        if removed == 0 {
            return;
        }
        if let Some(count) = self.counts.borrow_mut().get_mut(&id.event) {
            *count = count.saturating_sub(removed);
        }
        self.changed();
    }
}

pub struct GlobalEventBus;

thread_local! {
    static GLOBAL: RefCell<Rc<EventBus>> = RefCell::new(Rc::new(EventBus::of_kind(Bus::Global)));
}

impl GlobalEventBus {
    pub(crate) fn instance() -> Rc<EventBus> {
        GLOBAL.with(|bus| bus.borrow().clone())
    }

    /// Puts an empty global bus in place of this thread's, so a test starts
    /// with no subscriber a test before it left behind.
    #[cfg(feature = "test-utils")]
    pub fn replace_for_test() {
        GLOBAL.with(|bus| *bus.borrow_mut() = Rc::new(EventBus::of_kind(Bus::Global)));
    }

    /// The global bus, for reading what is subscribed to it.
    pub fn bus() -> Rc<EventBus> {
        Self::instance()
    }

    /// Publishes the event on the UI thread's global event bus.
    ///
    /// The global event bus lives on the UI thread, so this call is redirected
    /// there via the UI dispatcher. It is safe to call from any thread.
    pub fn publish<M: Event>(msg: M) {
        let cause = trace::current();
        invoke_on_ui(move || {
            let _resumed = trace::resume(cause);
            Self::instance().publish(msg);
        });
    }

    pub fn subscribe<A, M>(addr: Addr<A>) -> BusSubscription
    where
        A: Handler<M> + 'static,
        M: Event,
    {
        Self::instance().subscribe(addr)
    }

    #[track_caller]
    pub fn subscribe_fn<M: Event>(callback: impl Fn(M) + 'static) -> BusSubscription {
        Self::instance().subscribe_fn(callback)
    }

    /// See [`EventBus::answer_fn`].
    #[track_caller]
    pub fn answer_fn<Req: RpcCall>(
        answer: impl Fn(Req) -> Req::Response + 'static,
    ) -> BusSubscription {
        Self::instance().answer_fn(answer)
    }

    pub fn count_subscribers<M: Event>() -> usize {
        Self::instance().count_subscribers::<M>()
    }

    pub fn has_subscribers<M: Event>() -> bool {
        Self::instance().has_subscribers::<M>()
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as StdCell;

    #[derive(Clone)]
    struct Ping;
    impl Event for Ping {}

    #[derive(Clone)]
    struct Pong;
    impl Event for Pong {}

    #[test]
    fn a_bus_tells_a_watcher_when_what_is_subscribed_to_it_changes() {
        let bus = Rc::new(EventBus::for_root(3));
        let seen = Rc::new(RefCell::new(Vec::new()));

        let sink = seen.clone();
        let reading = bus.clone();
        changes::watch(move |change| {
            sink.borrow_mut().push((change.clone(), reading.subscriptions().len()));
        });

        let sub = bus.subscribe_fn(|_: Ping| {});
        drop(sub);
        changes::stop_watching();

        let changed = Change::Subscriptions {
            bus: Bus::Window,
            root: Some(3),
        };
        assert_eq!(
            *seen.borrow(),
            [(changed.clone(), 1), (changed, 0)],
            "told after the change, with the bus free to read"
        );
    }

    #[test]
    fn dropping_the_handle_ends_the_subscription() {
        let bus = Rc::new(EventBus::new());
        let seen = Rc::new(StdCell::new(0));

        let counter = seen.clone();
        let sub = bus.subscribe_fn(move |_: Ping| counter.set(counter.get() + 1));

        bus.publish(Ping);
        assert_eq!(seen.get(), 1);

        drop(sub);
        assert_eq!(bus.count_subscribers::<Ping>(), 0);

        bus.publish(Ping);
        assert_eq!(seen.get(), 1, "no delivery after the handle is dropped");
    }

    #[test]
    fn a_subscriber_may_subscribe_and_unsubscribe_while_it_is_being_told() {
        let bus = Rc::new(EventBus::new());
        let seen = Rc::new(StdCell::new(0));

        let later = Rc::new(RefCell::new(None));
        let kept = later.clone();
        let subscribing = bus.clone();
        let counter = seen.clone();

        // Two things a handler is allowed to do, both of which used to
        // borrow the map that the publication was walking.
        let sub = bus.subscribe_fn(move |_: Ping| {
            counter.set(counter.get() + 1);

            let counting = counter.clone();
            *kept.borrow_mut() = Some(subscribing.subscribe_fn(move |_: Pong| {
                counting.set(counting.get() + 10);
            }));
        });

        bus.publish(Ping);
        assert_eq!(seen.get(), 1);

        bus.publish(Pong);
        assert_eq!(seen.get(), 11, "what subscribed during the publication hears the next one");

        drop(sub);
        bus.publish(Ping);
        assert_eq!(seen.get(), 11, "and the one that went is not told again");
    }

    #[test]
    fn removal_only_touches_its_own_event() {
        let bus = Rc::new(EventBus::new());

        let ping = bus.subscribe_fn(|_: Ping| {});
        let _pong = bus.subscribe_fn(|_: Pong| {});

        drop(ping);

        assert_eq!(bus.count_subscribers::<Ping>(), 0);
        assert_eq!(bus.count_subscribers::<Pong>(), 1);
    }

    #[test]
    fn a_handle_from_one_bus_cannot_disturb_another() {
        let first = Rc::new(EventBus::new());
        let second = Rc::new(EventBus::new());

        let sub = first.subscribe_fn(|_: Ping| {});
        let _same_seq_elsewhere = second.subscribe_fn(|_: Ping| {});

        drop(sub);

        assert_eq!(first.count_subscribers::<Ping>(), 0);
        assert_eq!(
            second.count_subscribers::<Ping>(),
            1,
            "both subscriptions were the first on their own bus, and once shared a raw id"
        );
    }

    #[test]
    fn a_handle_outliving_its_bus_is_harmless() {
        let sub = {
            let bus = Rc::new(EventBus::new());
            bus.subscribe_fn(|_: Ping| {})
        };

        drop(sub);
    }

    #[test]
    fn leak_keeps_the_subscription() {
        let bus = Rc::new(EventBus::new());
        bus.subscribe_fn(|_: Ping| {}).leak();

        assert_eq!(bus.count_subscribers::<Ping>(), 1);
    }
}

#[cfg(feature = "test-utils")]
impl EventBus {
    pub fn queue_test_task(task: Box<dyn FnOnce() + Send>) {
        TEST_TASK_QUEUE.lock().unwrap().push(task);
    }
    pub fn process_queue() {
        let tasks: Vec<_> = std::mem::take(&mut *TEST_TASK_QUEUE.lock().unwrap());
        for task in tasks {
            task();
        }
    }

    pub fn is_queue_empty() -> bool {
        TEST_TASK_QUEUE.lock().unwrap().is_empty()
    }

    pub fn task_count() -> usize {
        ACTIVE_TASKS.load(std::sync::atomic::Ordering::SeqCst)
    }
}
