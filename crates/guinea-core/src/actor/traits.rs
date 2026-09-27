use crate::actor::Cx;
use crate::actor::event_bus::builder::EventSubscription;

pub trait Handler<M: 'static>: 'static {
    /// Where the handler was written; `#[handler]` fills it in.
    const DECLARED: Option<crate::actor::shape::Declared> = None;

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

pub trait AllowedSignal<M: 'static> {}
impl<M: 'static> AllowedSignal<M> for M {}
