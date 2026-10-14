use crate::actor::event_bus::subscribe::Event;
use crate::actor::{Addr, Handler};

/// Declares which messages a handler may send. Optional: an actor that
/// declares no graph gets [`Open`], which allows everything.
#[diagnostic::on_unimplemented(
    message = "handler `{M}` does not declare sending `{Out}`",
    label = "not declared",
    note = "add the edge in `actor! {{ handlers {{ {M} => {{ send {Out} }} }} }}`"
)]
pub trait Allows<M, Out> {}

/// The default: no graph declared, nothing restricted.
pub struct Open;

impl<M, Out> Allows<M, Out> for Open {}

/// What an actor's manifest subscribes to on the bus of its tree. `actor!` writes
/// it; the subscriptions are the actor's, and end when it is disposed.
#[diagnostic::on_unimplemented(
    message = "`{A}` cannot subscribe to `{Self}`: it is not an event, or `{A}` has no handler for it",
    label = "not an event `{A}` handles",
    note = "an event derives `#[derive(Clone, guinea::Event)]`; the handler is `#[handler] fn name(this: &mut {A}, event: {Self})`"
)]
pub trait EventSubscription<A> {
    fn subscribe_into(addr: &Addr<A>);
}

impl<A, M> EventSubscription<A> for M
where
    M: Event,
    A: Handler<M> + 'static,
{
    fn subscribe_into(addr: &Addr<A>) {
        addr.subscribe::<M>();
    }
}

impl<A> EventSubscription<A> for () {
    fn subscribe_into(_: &Addr<A>) {}
}
