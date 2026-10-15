use crate::actor::addr::Addr;
use crate::actor::cancel::Cancel;
use crate::actor::event_bus::subscribe::Event;
use crate::actor::event_bus::{RpcCall, RpcRequest};
use crate::actor::flow::Allows;
use crate::actor::traits::Handler;
use crate::actor::{ManagedActor, PublishedBy};
use std::marker::PhantomData;

mod async_ctx;
#[cfg(test)]
mod chain;
pub mod task;

pub use async_ctx::AsyncContext;
pub use task::Task;

/// What a handler may do besides change its actor: send on, publish, start
/// background work.
///
/// Typed by the message being handled, not holding it: the message is the
/// handler's own argument, and `M` is what `actor!`'s flow checks - a handler
/// of `Query` declared `Query => { task Found }` may spawn work that answers
/// `Found`, and nothing else.
pub struct Cx<A: 'static, M = ()> {
    pub(super) addr: Addr<A>,
    handling: PhantomData<fn() -> M>,
}

impl<A: 'static, M> Clone for Cx<A, M> {
    fn clone(&self) -> Self {
        Self::new(self.addr.clone())
    }
}

impl<A: 'static, M> Cx<A, M> {
    pub(crate) fn new(addr: Addr<A>) -> Self {
        Self {
            addr,
            handling: PhantomData,
        }
    }

    pub fn addr(&self) -> Addr<A> {
        self.addr.clone()
    }

    /// Sends to this actor's own queue, handled once the current message is.
    pub fn send<Out>(&self, msg: Out)
    where
        Out: 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
    {
        self.addr.send(msg);
    }

    pub fn publish<E>(&self, msg: E)
    where
        E: Event + PublishedBy<A>,
    {
        if let Some(bus) = self.addr.tree_bus() {
            bus.publish(msg);
        }
    }

    /// The actor's cancellation token, for work that has to end itself rather
    /// than be dropped at an await - closing a file, telling the other end.
    ///
    /// [`Cx::task`] already guards what it spawns with it.
    pub fn cancellation(&self) -> Cancel {
        self.addr.cancellation()
    }
}

impl<A: 'static, Req: RpcCall> Cx<A, RpcRequest<Req>> {
    /// The handler of a request, as the handler of what it asks.
    pub(crate) fn asked(self) -> Cx<A, Req> {
        Cx::new(self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::chain::{Chain, First};
    use super::*;
    use crate::actor::addr::TestHome;
    use crate::trace::{self, Point};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn send_from_a_handler_is_drained_by_the_same_queue() {
        let home = TestHome::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new(
            Chain { log: log.clone() },
            &home,
        );

        addr.send(First);

        assert_eq!(&*log.borrow(), &["first", "second"]);
    }

    #[test]
    fn a_watcher_hears_which_actor_handled_a_message() {
        use crate::observability::changes::{self, Change};

        let home = TestHome::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        changes::watch(move |change| {
            if let Change::ActorHandled { id } = change {
                sink.borrow_mut().push(*id);
            }
        });

        let addr = Addr::new(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            &home,
        );
        addr.send(First);
        changes::stop_watching();

        let id = addr.id();
        assert_eq!(*seen.borrow(), [id, id], "First, then the Second it sent");
    }

    #[test]
    fn a_chain_of_sends_is_traced_back_to_the_action_that_started_it() {
        use crate::trace::{Cause, Record, Trace};

        let home = TestHome::new();
        let seen = Rc::new(RefCell::new(Vec::<Record>::new()));
        let sink = seen.clone();
        trace::observe(move |trace| {
            if let Trace::Begin(record) | Trace::Mark(record) = trace {
                sink.borrow_mut().push(record.clone());
            }
        });

        let addr = Addr::new(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            &home,
        );
        {
            let _action = trace::enter(|| Point::Action { message: "First" });
            addr.send(First);
        }
        trace::stop_observing();

        let seen = seen.borrow();
        let find = |wanted: &dyn Fn(&Point) -> bool| -> &Record {
            seen.iter().find(|record| wanted(&record.point)).expect("recorded")
        };
        let parent = |record: &Record| -> Option<Cause> { record.parent };

        let action = find(&|p| matches!(p, Point::Action { .. }));
        let send_first = find(&|p| matches!(p, Point::Send { message, .. } if message.ends_with("First")));
        let handle_first =
            find(&|p| matches!(p, Point::Handle { message, .. } if message.ends_with("First")));
        let send_second =
            find(&|p| matches!(p, Point::Send { message, .. } if message.ends_with("Second")));
        let handle_second =
            find(&|p| matches!(p, Point::Handle { message, .. } if message.ends_with("Second")));

        assert_eq!(parent(send_first), Some(action.id));
        assert_eq!(parent(handle_first), Some(send_first.id));
        assert_eq!(parent(send_second), Some(handle_first.id));
        assert_eq!(parent(handle_second), Some(send_second.id));
    }

    #[test]
    fn disposing_an_actor_cancels_what_it_spawned() {
        let home = TestHome::new();
        let addr = Addr::new(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            &home,
        );

        let cx = Cx::<_, First>::new(addr.clone());
        let cancel = cx.cancellation();
        assert!(!cancel.is_cancelled());

        home.tree.remove();

        assert!(cancel.is_cancelled(), "teardown has to reach the tasks too");
    }
}
