use crate::actor::addr::Addr;
use crate::actor::event_bus::subscribe::{
    AnswerFn, BusSubscription, FnSubscriber, Letter, Subscriber, SubscriptionId, UntypedSubscriber,
};
use crate::actor::traits::Handler;
use crate::observability::changes::{self, Change};
use crate::scope::Scope;
use crate::short_type_name;
use crate::trace::{self, Point};
use crate::ui_thread::invoke_on_ui;
use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub mod rpc;
pub mod subscribe;
pub use rpc::{Reply, ReplyTo, RpcCall, RpcRequest};
pub use subscribe::Event;

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
    /// Each list is replaced, never changed in place: a publication holds the
    /// list it started with, so one that subscribes or unsubscribes while it
    /// is being told neither finds the map borrowed nor changes who hears
    /// this one.
    subscribers: RefCell<HashMap<TypeId, Rc<[Rc<dyn UntypedSubscriber>]>>>,
    counts: RefCell<HashMap<TypeId, usize>>,
    /// The one subscriber that answers each request type, by its `seq`.
    answerers: RefCell<HashMap<TypeId, (u64, &'static str)>>,
    next_id: Cell<u64>,
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
            .map_or(0, |subscribers| subscribers.values().map(|list| list.len()).sum());
        f.debug_struct("EventBus")
            .field("subscribers", &subscribers)
            .finish()
    }
}

impl EventBus {
    /// A bus of its own, outside any tree. A
    /// [`ScopeTree`](crate::scope::ScopeTree) makes the one its scopes share.
    pub fn new() -> Self {
        Self {
            subscribers: RefCell::new(HashMap::new()),
            counts: RefCell::new(HashMap::new()),
            answerers: RefCell::new(HashMap::new()),
            next_id: Cell::new(0),
        }
    }

    /// `addr` hears `M` on this bus until the subscription is dropped.
    ///
    /// ```
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::{Event, EventBus};
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    ///
    /// #[derive(Clone)]
    /// struct Add(u32);
    ///
    /// impl Event for Add {}
    ///
    /// #[derive(Debug)]
    /// struct Total(u32);
    ///
    /// impl Handler<Add> for Total {
    ///     fn handle(&mut self, Add(n): Add, _cx: Cx<Self, Add>) {
    ///         self.0 += n;
    ///     }
    /// }
    ///
    /// let tree = ScopeTree::new();
    /// let total = Addr::new(Total(0), &Home::new(tree.scope()));
    /// let bus = Rc::new(EventBus::new());
    ///
    /// let subscription = bus.subscribe::<Total, Add>(total.clone());
    /// bus.publish(Add(2));
    /// assert_eq!(total.debug_snapshot(), "Total(\n    2,\n)");
    ///
    /// drop(subscription);
    /// bus.publish(Add(3));
    /// assert_eq!(total.debug_snapshot(), "Total(\n    2,\n)", "heard nothing once unsubscribed");
    /// ```
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

    /// `callback` hears `M` on this bus until the subscription is dropped.
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::{Event, EventBus};
    ///
    /// #[derive(Clone)]
    /// struct Add(u32);
    ///
    /// impl Event for Add {}
    ///
    /// let bus = Rc::new(EventBus::new());
    /// let total = Rc::new(Cell::new(0));
    ///
    /// let adding = total.clone();
    /// let subscription = bus.subscribe_fn(move |Add(n)| adding.set(adding.get() + n));
    /// bus.publish(Add(2));
    /// assert_eq!(total.get(), 2);
    ///
    /// drop(subscription);
    /// bus.publish(Add(3));
    /// assert_eq!(total.get(), 2, "heard nothing once unsubscribed");
    /// ```
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

    /// Answers `Req` with `answer` until the subscription is dropped: the
    /// callback counterpart of an actor's handler that returns the reply.
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

    /// Tells everyone subscribed to `M`, in the order they subscribed, before
    /// it returns. Who subscribes or goes while they are being told does not
    /// change who hears this one.
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::{Event, EventBus};
    ///
    /// #[derive(Clone)]
    /// struct Saved;
    ///
    /// impl Event for Saved {}
    ///
    /// let bus = Rc::new(EventBus::new());
    /// let heard = Rc::new(RefCell::new(Vec::new()));
    /// let late = Rc::new(RefCell::new(None));
    ///
    /// let (first, joining, joined) = (heard.clone(), bus.clone(), heard.clone());
    /// let _first = bus.subscribe_fn(move |_: Saved| {
    ///     first.borrow_mut().push("first");
    ///     let joined = joined.clone();
    ///     *late.borrow_mut() = Some(joining.subscribe_fn(move |_: Saved| joined.borrow_mut().push("late")));
    /// });
    /// let second = heard.clone();
    /// let _second = bus.subscribe_fn(move |_: Saved| second.borrow_mut().push("second"));
    ///
    /// bus.publish(Saved);
    /// assert_eq!(*heard.borrow(), ["first", "second"], "the one that joined meanwhile heard nothing");
    /// ```
    pub fn publish<M: Event>(&self, msg: M) {
        let _published = trace::enter(|| Point::Publish {
            event: short_type_name::<M>(),
            subscribers: self.count_subscribers::<M>(),
        });

        // Told from the list as it was when the publication began, not from
        // the map: a handler is free to subscribe or unsubscribe while it is
        // being told, and either would borrow the map this delivery is
        // walking. One that goes mid-publication still hears this one out.
        let Some(telling) = self.subscribers.borrow().get(&TypeId::of::<M>()).cloned() else {
            return;
        };

        let mut letter = Letter::new(msg);
        for (at, sub) in telling.iter().enumerate() {
            if at + 1 == telling.len() {
                letter.to_the_last();
            }

            // Around each one, not around the lot: the copy every subscriber
            // is handed is made as it opens the letter, and time spent making
            // it belongs to the delivery that needed it rather than to the
            // publication as a whole. Without this it is the difference
            // between a publication and the sum of its handlers, which is to
            // say invisible.
            let _delivered = trace::enter(|| Point::Deliver {
                event: short_type_name::<M>(),
            });

            sub.deliver(&mut letter);
        }
    }

    /// How many hear `M` on this bus now.
    ///
    /// ```
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::{Event, EventBus};
    ///
    /// #[derive(Clone)]
    /// struct Saved;
    ///
    /// impl Event for Saved {}
    ///
    /// let bus = Rc::new(EventBus::new());
    /// assert_eq!(bus.count_subscribers::<Saved>(), 0);
    ///
    /// let one = bus.subscribe_fn(|_: Saved| {});
    /// let other = bus.subscribe_fn(|_: Saved| {});
    /// assert_eq!(bus.count_subscribers::<Saved>(), 2);
    ///
    /// drop(one);
    /// assert_eq!(bus.count_subscribers::<Saved>(), 1);
    /// # drop(other);
    /// ```
    pub fn count_subscribers<M: Event>(&self) -> usize {
        let type_id = TypeId::of::<M>();
        *self.counts.borrow().get(&type_id).unwrap_or(&0)
    }

    /// Whether anyone hears `M` on this bus now.
    ///
    /// ```
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::{Event, EventBus};
    ///
    /// #[derive(Clone)]
    /// struct Saved;
    ///
    /// impl Event for Saved {}
    ///
    /// let bus = Rc::new(EventBus::new());
    /// assert!(!bus.has_subscribers::<Saved>());
    ///
    /// let subscription = bus.subscribe_fn(|_: Saved| {});
    /// assert!(bus.has_subscribers::<Saved>());
    ///
    /// drop(subscription);
    /// assert!(!bus.has_subscribers::<Saved>());
    /// ```
    pub fn has_subscribers<M: Event>(&self) -> bool {
        self.count_subscribers::<M>() > 0
    }
}

impl EventBus {
    /// The event types something is subscribed to, with how many subscribers
    /// each has.
    #[doc(hidden)]
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
    #[doc(hidden)]
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

    fn changed(&self) {
        changes::changed(|| Change::Subscriptions);
    }

    fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
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
                    "{answerer} answers {}, and {already} already does: a request has exactly \
                     one answerer. Make one of them only hear it - a handler that returns \
                     nothing - or answer from one place.",
                    subscriber.event(),
                );
            }
            answerers.insert(event, (id.seq, answerer));
        }

        *self.counts.borrow_mut().entry(event).or_insert(0) += 1;
        {
            let mut subscribers = self.subscribers.borrow_mut();
            let list = subscribers.entry(event).or_insert_with(|| Rc::from([]));
            *list = list.iter().cloned().chain([Rc::from(subscriber)]).collect();
        }
        self.changed();

        BusSubscription {
            bus: Rc::downgrade(self),
            id,
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

        let (removed, gone) = {
            let mut subscribers = self.subscribers.borrow_mut();
            let Some(list) = subscribers.get_mut(&id.event) else {
                return;
            };

            let kept: Rc<[Rc<dyn UntypedSubscriber>]> =
                list.iter().filter(|sub| sub.seq() != id.seq).cloned().collect();
            (list.len() - kept.len(), std::mem::replace(list, kept))
        };
        drop(gone);

        if removed == 0 {
            return;
        }
        if let Some(count) = self.counts.borrow_mut().get_mut(&id.event) {
            *count = count.saturating_sub(removed);
        }
        self.changed();
    }
}

impl Scope {
    /// Publishes `msg` on this scope's [`bus`](Scope::bus) from whichever
    /// thread this runs on: on the UI thread, where the bus lives, with its
    /// cause, and not at all once the tree is gone.
    pub fn publish<M: Event>(self, msg: M) {
        let cause = trace::current();
        invoke_on_ui(move || {
            let _resumed = trace::resume(cause);
            if let Some(bus) = self.tree_bus() {
                bus.publish(msg);
            }
        });
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

    struct Copied(Arc<std::sync::atomic::AtomicUsize>);

    impl Clone for Copied {
        fn clone(&self) -> Self {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Self(self.0.clone())
        }
    }

    impl Event for Copied {}

    #[test]
    fn a_subscriber_that_ends_another_subscription_as_it_goes_finds_the_bus_free() {
        let bus = Rc::new(EventBus::new());
        let inner = bus.subscribe_fn(|_: Pong| {});
        let outer = bus.subscribe_fn(move |_: Ping| {
            let _ = &inner;
        });

        let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(outer)));

        assert_eq!((dropped.is_ok(), bus.subscriptions()), (true, Vec::new()));
    }

    #[test]
    fn every_subscriber_but_the_last_is_handed_a_copy_and_the_last_the_event_itself() {
        let bus = Rc::new(EventBus::new());
        let _subscriptions: Vec<BusSubscription> =
            (0..3).map(|_| bus.subscribe_fn(|_: Copied| {})).collect();
        let copies = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        bus.publish(Copied(copies.clone()));

        assert_eq!(copies.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn a_bus_tells_a_watcher_when_what_is_subscribed_to_it_changes() {
        let bus = Rc::new(EventBus::new());
        let seen = Rc::new(RefCell::new(Vec::new()));

        let sink = seen.clone();
        let reading = bus.clone();
        changes::watch(move |change| {
            sink.borrow_mut().push((change.clone(), reading.subscriptions().len()));
        });

        let sub = bus.subscribe_fn(|_: Ping| {});
        drop(sub);
        changes::stop_watching();

        assert_eq!(
            *seen.borrow(),
            [(Change::Subscriptions, 1), (Change::Subscriptions, 0)],
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
