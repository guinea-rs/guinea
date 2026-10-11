use super::Cx;
use crate::actor::addr::Reach;
use crate::actor::flow::Allows;
use crate::actor::traits::Handler;
use crate::actor::{ManagedActor, short_type_name};
use crate::trace::{self, Cause, Point};
use crate::ui_thread::invoke_on_ui;
use futures_core::Stream;
use std::time::Instant;

/// One source as the trace sees it: whose it is, what its items come as, and
/// when it opened.
#[derive(Clone, Copy)]
struct Feed {
    actor: &'static str,
    actor_id: u64,
    output: &'static str,
    started: Instant,
}

impl Feed {
    fn new<A: 'static>(actor_id: usize, output: &'static str) -> Self {
        Self {
            actor: short_type_name::<A>(),
            actor_id: actor_id as u64,
            output,
            started: Instant::now(),
        }
    }

    fn opened(&self) -> Point {
        Point::Source {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
        }
    }

    fn arrived(&self, source: Cause) -> Point {
        Point::Arrived {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            source: source.get(),
        }
    }

    fn pulled(&self, source: Cause) -> Point {
        Point::Pull {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            source: source.get(),
        }
    }

    /// Opens a pull of the source: what the stream does until it has its next
    /// item, runs dry or is dropped is recorded under it.
    fn pull(self, opened: Cause) -> Pulling {
        let id = trace::reserve();
        crate::observability::begin_anywhere(id, None, move || self.pulled(opened));

        Pulling {
            id,
            started: Instant::now(),
        }
    }

    fn closed(self, opened: Cause, gone: bool) {
        let point = Point::Closed {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            took_us: self.started.elapsed().as_micros() as u64,
            gone,
        };
        let _resumed = trace::resume(Some(opened));

        crate::observability::mark_anywhere(move || point);
    }
}

/// A pull that is open, closed when it is dropped: when the item comes, or
/// when the source is dropped waiting for it.
struct Pulling {
    id: Cause,
    started: Instant,
}

impl Drop for Pulling {
    fn drop(&mut self) {
        crate::observability::end_anywhere(self.id, self.started.elapsed());
    }
}

impl<A: 'static, M> Cx<A, M> {
    /// Feeds this actor what `source` yields, each item as `into(item)`, for
    /// as long as the actor lives: a watch, a subscription, a pipe that
    /// pushes.
    ///
    /// A source is not work of what opened it, as a timer is not work of
    /// what started it. The handler that opens it is done once it is open,
    /// and each item arrives as a root of its own - so an action that opens
    /// a watch is finished when the watch is open, not when it runs dry.
    /// What the stream does to make an item - a publish from inside its poll,
    /// say - is recorded under a [`Point::Pull`] of the source, a root as
    /// well.
    ///
    /// Declared as `bg` in `actor!`, like any other work that answers later.
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::pin::Pin;
    /// use std::rc::Rc;
    /// use std::task::{Context, Poll};
    /// use std::time::Duration;
    ///
    /// use guinea_core::actor::{Addr, Cx, Handler, Home, Stream};
    /// use guinea_core::scope::ScopeTree;
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// struct Countdown(u32);
    ///
    /// impl Stream for Countdown {
    ///     type Item = u32;
    ///
    ///     fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<u32>> {
    ///         if self.0 == 0 {
    ///             return Poll::Ready(None);
    ///         }
    ///         self.0 -= 1;
    ///         Poll::Ready(Some(self.0 + 1))
    ///     }
    /// }
    ///
    /// #[derive(Debug)]
    /// struct Launch(Rc<RefCell<Vec<u32>>>);
    ///
    /// struct Start;
    ///
    /// struct Tick(u32);
    ///
    /// guinea_macros::actor! {
    ///     Launch {
    ///         handlers {
    ///             Start => { bg Tick }
    ///             Tick
    ///         }
    ///     }
    /// }
    ///
    /// impl Handler<Start> for Launch {
    ///     fn handle(&mut self, _: Start, cx: Cx<Self, Start>) {
    ///         cx.spawn_source(Countdown(3), Tick);
    ///     }
    /// }
    ///
    /// impl Handler<Tick> for Launch {
    ///     fn handle(&mut self, Tick(n): Tick, _cx: Cx<Self, Tick>) {
    ///         self.0.borrow_mut().push(n);
    ///     }
    /// }
    ///
    /// let ui = UiLoop::install();
    /// let runtime = tokio::runtime::Runtime::new().unwrap();
    /// let _entered = runtime.enter();
    ///
    /// let tree = ScopeTree::new();
    /// let heard = Rc::new(RefCell::new(Vec::new()));
    /// let launch = Addr::new_managed(Launch(heard.clone()), &Home::new(tree.scope(), None));
    ///
    /// launch.send(Start);
    /// assert!(heard.borrow().is_empty(), "the source is not drained by the send");
    ///
    /// assert!(ui.run_until(|| heard.borrow().len() == 3, Duration::from_secs(5)));
    /// assert_eq!(*heard.borrow(), [3, 2, 1]);
    /// ```
    pub fn spawn_source<S, Out, F>(&self, source: S, into: F)
    where
        S: Stream + Send + 'static,
        Out: Send + 'static,
        F: FnMut(S::Item) -> Out + Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
    {
        let reach = self.addr.reach();
        let cancel = self.addr.cancellation();
        let feed = Feed::new::<A>(reach.id(), short_type_name::<Out>());
        let opened = trace::mark(|| feed.opened());

        crate::executor::spawn(async move {
            let pouring = trace::within(None, pour(reach, source, into, feed, opened));
            let ran_dry = cancel.guard(pouring).await.is_some();

            feed.closed(opened, !ran_dry || cancel.is_cancelled());
        });
    }
}

/// Hands each item of `source` to the UI thread, as a root of its own, until
/// the source runs dry. Gives way after each item, so a source that is always
/// ready still notices its actor going.
async fn pour<A, S, Out, F>(reach: Reach<A>, source: S, mut into: F, feed: Feed, opened: Cause)
where
    A: Handler<Out> + 'static,
    S: Stream,
    Out: Send + 'static,
    F: FnMut(S::Item) -> Out,
{
    let mut source = std::pin::pin!(source);

    loop {
        let next = async {
            let pulling = feed.pull(opened);

            std::future::poll_fn(|cx| {
                let _pulling = trace::resume(Some(pulling.id));
                source.as_mut().poll_next(cx)
            })
            .await
        };
        let Some(item) = next.await else {
            break;
        };
        let message = into(item);

        invoke_on_ui(move || {
            let _root = trace::resume(None);
            let _arrived = trace::enter(|| feed.arrived(opened));

            if let Some(addr) = reach.addr() {
                addr.send(message);
            }
        });

        crate::executor::yield_now().await;
    }
}
