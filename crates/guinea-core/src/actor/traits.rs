use crate::actor::flow::EventSubscription;
use crate::actor::shape::{Declared, Shape};
use crate::actor::{Addr, Cx};
use crate::inspect::Inspector;

#[diagnostic::on_unimplemented(
    message = "`{Self}` has no handler for `{M}`",
    label = "no handler for `{M}`",
    note = "write one: `#[handler] fn name(this: &mut {Self}, msg: {M}) {{ .. }}`"
)]
pub trait Handler<M: 'static>: 'static {
    /// Where the handler was written; `#[handler]` fills it in.
    const DECLARED: Option<Declared> = None;

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
    /// Declared outgoing messages per handler, or `Open` when undeclared.
    type Flow;
    /// What `actor!` declared, for devtools.
    const SHAPE: Shape = Shape::UNKNOWN;

    /// How devtools show its state; `actor!` writes it.
    #[doc(hidden)]
    fn show(&self, at: Inspector<'_>);
}

/// What can be handed an `M`: a function that takes it, or an actor that
/// handles it.
pub trait Receives<M>: 'static {
    fn receive(&self, message: M);
}

impl<M, F: Fn(M) + 'static> Receives<M> for F {
    fn receive(&self, message: M) {
        self(message)
    }
}

impl<M: 'static, A: Handler<M>> Receives<M> for Addr<A> {
    fn receive(&self, message: M) {
        self.send(message)
    }
}

/// An event `A` declares publishing; `actor!` writes it from `publishes`.
#[diagnostic::on_unimplemented(
    message = "`{A}` does not declare publishing `{Self}`",
    label = "not in `publishes`",
    note = "add it to the manifest: `actor! {{ {A} {{ publishes {{ {Self} }} }} }}`"
)]
pub trait PublishedBy<A> {}
