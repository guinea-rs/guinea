use crate::actor::event_bus::subscribe::Event;
use crate::actor::{Addr, Cx};
use crate::trace::Bus;

pub trait Handler<M: 'static>: 'static {
    /// Where the handler was written; `#[handler]` fills it in.
    const DECLARED: Option<crate::actor::shape::Declared> = None;

    /// Whether handling `M` answers it: set for a request's handler, the one
    /// that returns the reply. A bus lets one subscriber answer each request
    /// type; the rest only hear it.
    const ANSWERS: bool = false;

    fn handle(&mut self, msg: M, cx: Cx<Self, M>)
    where
        Self: Sized;
}

pub trait ManagedActor: Sized + 'static {
    type Bus: EventSubscription<Self>;
    type Signals;
    /// Declared outgoing messages per handler, or `Open` when undeclared.
    type Flow;
    /// What `actor!` declared, for devtools.
    const SHAPE: crate::actor::shape::Shape = crate::actor::shape::Shape::UNKNOWN;
}

/// What an actor's manifest subscribes to on the global bus. `actor!` writes
/// it; the subscriptions are the actor's, and end when it is disposed.
pub trait EventSubscription<A> {
    fn subscribe_into(addr: &Addr<A>);
}

impl<A, M> EventSubscription<A> for M
where
    M: Event,
    A: Handler<M> + 'static,
{
    fn subscribe_into(addr: &Addr<A>) {
        addr.subscribe_on::<M>(Bus::Global);
    }
}

impl<A> EventSubscription<A> for () {
    fn subscribe_into(_: &Addr<A>) {}
}

pub trait AllowedSignal<M: 'static> {}
impl<M: 'static> AllowedSignal<M> for M {}
