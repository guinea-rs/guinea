use crate::actor::addr::Addr;
use crate::actor::event_bus::rpc::{AsyncBus, RpcCall, RpcRequest};
use crate::actor::short_type_name;
use crate::actor::traits::Handler;
use crate::trace::Bus;

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Weak;

use super::{EventBus, HeardBy};

/// Identifies one subscription on one bus. Carries the event's `TypeId` so
/// removal goes straight to the right bucket instead of scanning every one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SubscriptionId {
    pub(super) seq: u64,
    pub(super) event: TypeId,
}

/// Undoes a subscription when dropped.
///
/// Holds the bus weakly: a live subscription never keeps its bus alive, and a
/// handle outliving its bus is a no-op rather than a panic. There is no way to
/// get the raw id out, so an unsubscribe cannot be forgotten - to deliberately
/// keep a subscription for the rest of the process, say [`Self::leak`].
pub struct BusSubscription {
    pub(super) bus: Weak<EventBus>,
    pub(super) id: SubscriptionId,
}

impl BusSubscription {
    /// Keeps the subscription alive for the rest of the process.
    pub fn leak(self) {
        std::mem::forget(self);
    }
}

impl Drop for BusSubscription {
    fn drop(&mut self) {
        if let Some(bus) = self.bus.upgrade() {
            bus.remove(self.id);
        }
    }
}

impl crate::scope::Teardown for BusSubscription {
    fn teardown(self) {
        drop(self);
    }
}

/// A type that travels over the global bus, where subscribers find it by its
/// `TypeId` alone.
///
/// Implemented by hand or with `#[derive(Event)]`, never for free: the orphan
/// rule then keeps `String`, `u32` and `()` off the bus, since two features
/// that each published a type nobody owns would receive each other's events.
pub trait Event: Clone + Send + 'static {
    /// What a subscriber that only hears this is handed: the event itself,
    /// except for a request, which its listeners must not be able to answer.
    #[doc(hidden)]
    fn overheard(self) -> Self {
        self
    }
}

pub trait UntypedSubscriber: 'static {
    fn deliver(&self, msg: Box<dyn Any>, bus: Bus);
    fn seq(&self) -> u64;
    fn event(&self) -> &'static str;

    /// Who it is, when it answers what it hears rather than only hearing it.
    fn answerer(&self) -> Option<&'static str> {
        None
    }

    /// Whether it lives in a scope that is asleep, and hears nothing now.
    fn is_asleep(&self) -> bool {
        false
    }

    /// Who it is, for [`EventBus::listeners`].
    fn heard_by(&self) -> HeardBy {
        HeardBy::Unknown
    }
}

pub struct Subscriber<A: Handler<M>, M: Event> {
    pub(super) seq: u64,
    pub(super) addr: Addr<A>,
    pub(super) _marker: PhantomData<M>,
}

impl<A, M> UntypedSubscriber for Subscriber<A, M>
where
    A: Handler<M> + 'static,
    M: Event,
{
    fn deliver(&self, msg: Box<dyn Any>, _bus: Bus) {
        if self.addr.is_asleep() {
            return;
        }

        if let Ok(concrete_msg) = msg.downcast::<M>() {
            let message = match <A as Handler<M>>::ANSWERS {
                true => *concrete_msg,
                false => concrete_msg.overheard(),
            };
            self.addr.send(message);
        }
    }

    fn seq(&self) -> u64 {
        self.seq
    }

    fn event(&self) -> &'static str {
        short_type_name::<M>()
    }

    fn answerer(&self) -> Option<&'static str> {
        <A as Handler<M>>::ANSWERS.then(short_type_name::<A>)
    }

    fn is_asleep(&self) -> bool {
        self.addr.is_asleep()
    }

    fn heard_by(&self) -> HeardBy {
        HeardBy::Actor {
            name: short_type_name::<A>(),
            id: self.addr.id(),
        }
    }
}

/// A callback that answers `Req`: see [`EventBus::answer_fn`].
pub struct AnswerFn<Req: RpcCall> {
    pub(super) seq: u64,
    pub(super) answer: Box<dyn Fn(Req) -> Req::Response>,
    pub(super) at: &'static Location<'static>,
}

impl<Req: RpcCall> UntypedSubscriber for AnswerFn<Req> {
    fn deliver(&self, msg: Box<dyn Any>, _bus: Bus) {
        if let Ok(request) = msg.downcast::<RpcRequest<Req>>() {
            let RpcRequest {
                correlation_id,
                payload,
                ..
            } = *request;
            AsyncBus::reply(correlation_id, (self.answer)(payload));
        }
    }

    fn seq(&self) -> u64 {
        self.seq
    }

    fn event(&self) -> &'static str {
        short_type_name::<RpcRequest<Req>>()
    }

    fn answerer(&self) -> Option<&'static str> {
        Some("a callback")
    }

    fn heard_by(&self) -> HeardBy {
        HeardBy::Callback(self.at)
    }
}

pub struct FnSubscriber<M: Event> {
    pub(super) seq: u64,
    pub(super) callback: std::sync::Arc<dyn Fn(M) + 'static>,
    pub(super) at: &'static Location<'static>,
}

impl<M: Event> UntypedSubscriber for FnSubscriber<M> {
    /// No `Deliver` of its own: the publication marks one around every
    /// subscriber, so one here would be the same span twice.
    fn deliver(&self, msg: Box<dyn Any>, _bus: Bus) {
        if let Ok(concrete_msg) = msg.downcast::<M>() {
            (self.callback)(concrete_msg.overheard());
        }
    }

    fn seq(&self) -> u64 {
        self.seq
    }

    fn event(&self) -> &'static str {
        short_type_name::<M>()
    }

    fn heard_by(&self) -> HeardBy {
        HeardBy::Callback(self.at)
    }
}
