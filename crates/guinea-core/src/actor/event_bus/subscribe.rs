use crate::actor::addr::Addr;
use crate::actor::short_type_name;
use crate::actor::traits::Handler;
use crate::trace::Bus;

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::rc::Weak;

use super::EventBus;

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
pub trait Event: Clone + Send + 'static {}

pub trait UntypedSubscriber: 'static {
    fn deliver(&self, msg: Box<dyn Any>, bus: Bus);
    fn seq(&self) -> u64;
    fn event(&self) -> &'static str;
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
            self.addr.send(*concrete_msg);
        }
    }

    fn seq(&self) -> u64 {
        self.seq
    }

    fn event(&self) -> &'static str {
        short_type_name::<M>()
    }
}

pub struct FnSubscriber<M: Event> {
    pub(super) seq: u64,
    pub(super) callback: std::sync::Arc<dyn Fn(M) + 'static>,
}

impl<M: Event> UntypedSubscriber for FnSubscriber<M> {
    /// No `Deliver` of its own: the publication marks one around every
    /// subscriber, so one here would be the same span twice.
    fn deliver(&self, msg: Box<dyn Any>, _bus: Bus) {
        if let Ok(concrete_msg) = msg.downcast::<M>() {
            (self.callback)(*concrete_msg);
        }
    }

    fn seq(&self) -> u64 {
        self.seq
    }

    fn event(&self) -> &'static str {
        short_type_name::<M>()
    }
}
