use super::Cx;
use crate::actor::addr::Reach;
use crate::actor::cancel::Cancel;
use crate::actor::event_bus::subscribe::Event;
use crate::actor::event_bus::{EventBus, GlobalEventBus};
use crate::actor::traits::Handler;
use crate::actor::{AllowedSignal, ManagedActor, invoke_on_ui};
use crate::trace;

pub struct AsyncContext<A: 'static> {
    reach: Reach<A>,
    cancel: Cancel,
}

impl<A: 'static> Clone for AsyncContext<A> {
    fn clone(&self) -> Self {
        Self {
            reach: self.reach,
            cancel: self.cancel.clone(),
        }
    }
}

impl<A: 'static> AsyncContext<A> {
    pub(crate) fn new(reach: Reach<A>, cancel: Cancel) -> Self {
        Self { reach, cancel }
    }

    /// The actor's cancellation token, to hand to something that takes one.
    pub fn cancellation(&self) -> Cancel {
        self.cancel.clone()
    }

    /// Whether the actor is gone. Worth asking between steps of long work:
    /// what comes after is for nobody.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Waits until the actor is gone - one arm of a `select!`, for work that
    /// has to end itself rather than be dropped at an await.
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    /// `fut`'s output, or `None` if the actor went away first.
    pub async fn until_gone<F: Future>(&self, fut: F) -> Option<F::Output> {
        self.cancel.guard(fut).await
    }

    /// Publishes `msg` on the global bus, from whichever thread this runs on:
    /// the bus lives on the UI thread, and the event goes there with its
    /// cause.
    pub fn publish<M>(&self, msg: M)
    where
        A: ManagedActor,
        M: Event,
        A::Signals: AllowedSignal<M>,
    {
        GlobalEventBus::publish(msg);
    }

    pub fn publish_local<M>(&self, bus: &EventBus, msg: M)
    where
        A: ManagedActor,
        M: Event,
        A::Signals: AllowedSignal<M>,
    {
        bus.publish(msg);
    }

    pub fn send<M>(&self, msg: M)
    where
        M: Send + 'static,
        A: Handler<M>,
    {
        let reach = self.reach;
        let cause = trace::current();

        invoke_on_ui(move || {
            if let Some(addr) = reach.addr() {
                addr.send_under(msg, cause);
            }
        });
    }
}

impl<A: 'static, M> Cx<A, M> {
    pub fn async_ctx(&self) -> AsyncContext<A> {
        AsyncContext::new(self.addr.reach(), self.addr.cancellation())
    }
}
