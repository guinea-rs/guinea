use crate::actor::addr::Addr;
use crate::actor::traits::Handler;
use crate::actor::{Cx, short_type_name};
use crate::trace::{self, Cause, Point};

pub trait Envelope<A> {
    fn handle(&mut self, actor: &mut A, addr: &Addr<A>);
}

pub struct MessageEnvelope<M: 'static> {
    pub(super) message: Option<M>,
    /// The send that queued this message.
    pub(super) cause: Cause,
}

impl<A, M: 'static> Envelope<A> for MessageEnvelope<M>
where
    A: Handler<M>,
{
    fn handle(&mut self, actor: &mut A, addr: &Addr<A>) {
        if let Some(message) = self.message.take() {
            let _handling = trace::enter_under(Some(self.cause), || Point::Handle {
                actor: short_type_name::<A>(),
                message: short_type_name::<M>(),
            });
            actor.handle(message, Cx::new(addr.clone()));
        }
    }
}
