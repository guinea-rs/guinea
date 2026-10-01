use crate::actor::addr::{Addr, registered};
use crate::actor::cancel::Cancel;
use crate::actor::event_bus::{EventBus, GlobalEventBus};
use crate::actor::event_bus::subscribe::Event;
use crate::actor::traits::Handler;
use crate::actor::{AllowedSignal, ManagedActor, invoke_on_ui, short_type_name};
use crate::trace::{self, Cause, Point};
use futures_core::Stream;
use std::marker::PhantomData;
use std::time::Instant;
use tokio::sync::oneshot;

/// What a handler may do besides change its actor: send on, publish, start
/// background work.
///
/// Typed by the message being handled, not holding it: the message is the
/// handler's own argument, and `M` is what `actor!`'s flow checks - a handler
/// of `Query` declared `Query => { bg Found }` may spawn work that answers
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

/// One background task as the trace sees it: whose it is, what it owes them,
/// and when it started.
///
/// The actor's id is what ties the task to a segment: a task belongs where
/// its actor does, and the snapshot already says where that is.
#[derive(Clone, Copy)]
struct Task {
    actor: &'static str,
    actor_id: u64,
    output: &'static str,
    started: Instant,
}

impl Task {
    fn new<A: 'static>(actor_id: usize, output: &'static str) -> Self {
        Self {
            actor: short_type_name::<A>(),
            actor_id: actor_id as u64,
            output,
            started: Instant::now(),
        }
    }

    fn spawn(&self) -> Point {
        Point::Spawn {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
        }
    }

    fn settled(&self) -> Point {
        Point::Settled {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            took_us: self.took_us(),
        }
    }

    fn cancelled(&self) -> Point {
        Point::Cancelled {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            took_us: self.took_us(),
        }
    }

    fn took_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    /// Records the end of the task under its spawn, from whichever thread
    /// polled it last: [`crate::devtools::mark_anywhere`] carries it to the
    /// thread devtools watch.
    fn ended(self, spawned: Cause, point: Point) {
        let _resumed = trace::resume(Some(spawned));

        crate::devtools::mark_anywhere(move || point);
    }
}

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

    fn closed(self, opened: Cause, gone: bool) {
        let point = Point::Closed {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            took_us: self.started.elapsed().as_micros() as u64,
            gone,
        };
        let _resumed = trace::resume(Some(opened));

        crate::devtools::mark_anywhere(move || point);
    }
}

impl<A: 'static, M> Cx<A, M> {
    pub(crate) fn new(addr: Addr<A>) -> Self {
        Self {
            addr,
            handling: PhantomData,
        }
    }

    /// The same actor, as the handler of another message.
    pub(crate) fn handling<N>(self) -> Cx<A, N> {
        Cx::new(self.addr)
    }

    pub fn addr(&self) -> Addr<A> {
        self.addr.clone()
    }

    /// The same actor, handling nothing in particular.
    pub fn detach(&self) -> Cx<A, ()> {
        Cx::new(self.addr.clone())
    }

    /// Sends to this actor's own queue; drained by the same `process_queue`.
    pub fn send<Out>(&self, msg: Out)
    where
        Out: 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: crate::actor::flow::Allows<M, Out>,
    {
        self.addr.send(msg);
    }

    pub fn publish<E>(&self, msg: E)
    where
        A: ManagedActor,
        E: Event,
        A::Signals: AllowedSignal<E>,
    {
        GlobalEventBus::instance().publish(msg);
    }

    pub fn publish_local<E>(&self, bus: &EventBus, msg: E)
    where
        A: ManagedActor,
        E: Event,
        A::Signals: AllowedSignal<E>,
    {
        bus.publish(msg);
    }

    /// The actor's cancellation token, for work that has to end itself rather
    /// than be dropped at an await - closing a file, telling the other end.
    ///
    /// [`Cx::spawn_bg`] already guards what it spawns with it.
    pub fn cancellation(&self) -> Cancel {
        self.addr.cancellation()
    }

    /// Runs `fut` off the UI thread and sends what it returns back to this
    /// actor.
    ///
    /// The task lives as long as the actor: once the actor is disposed, `fut`
    /// is dropped where it last awaited and nothing is sent.
    pub fn spawn_bg<Out, Fut>(&self, fut: Fut)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: crate::actor::flow::Allows<M, Out>,
        Fut: Future<Output = Out> + 'static + Send,
    {
        self.bg(fut, false);
    }

    /// [`Cx::spawn_bg`], with `listens` for work that was handed the
    /// token and winds itself down: it is left to finish instead of being
    /// dropped at its next await. Either way nothing is sent to an actor
    /// that is gone.
    fn bg<Out, Fut>(&self, fut: Fut, listens: bool)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: crate::actor::flow::Allows<M, Out>,
        Fut: Future<Output = Out> + 'static + Send,
    {
        let id = self.addr.id;
        let cancel = self.addr.cancellation();
        let task = Task::new::<A>(id, short_type_name::<Out>());
        let spawned = trace::mark(|| task.spawn());

        #[cfg(feature = "test-utils")]
        let counted = crate::actor::event_bus::Counted::new();

        crate::executor::spawn(async move {
            let running = trace::within(Some(spawned), fut);
            let result = match listens {
                true => Some(running.await),
                false => cancel.guard(running).await,
            };

            let Some(result) = result.filter(|_| !cancel.is_cancelled()) else {
                task.ended(spawned, task.cancelled());
                return;
            };

            let return_task = move || {
                #[cfg(feature = "test-utils")]
                let _counted = counted;

                let settled = trace::mark_under(Some(spawned), || task.settled());

                if let Some(addr) = registered::<A>(id) {
                    addr.send_under(result, Some(settled));
                }
            };

            invoke_on_ui(return_task);
        });
    }

    /// [`Cx::spawn_bg`] for work that wants to hear the cancellation
    /// rather than be dropped by it: a loop that checks between rounds, a
    /// connection that says goodbye, a `select!` arm.
    ///
    /// ```ignore
    /// cx.spawn_bg_with::<Tick, _, _>(|gone| async move {
    ///     while !gone.is_cancelled() {
    ///         poll().await;
    ///     }
    ///     Tick
    /// });
    /// ```
    pub fn spawn_bg_with<Out, Fut, F>(&self, work: F)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: crate::actor::flow::Allows<M, Out>,
        Fut: Future<Output = Out> + 'static + Send,
        F: FnOnce(Cancel) -> Fut,
    {
        self.bg(work(self.addr.cancellation()), true);
    }

    /// [`Cx::spawn_bg`] for work that answers no one; cancelled with the
    /// actor just the same.
    pub fn spawn_bg_detached<Fut>(&self, fut: Fut)
    where
        Fut: Future<Output = ()> + 'static + Send,
    {
        self.bg_detached(fut, false);
    }

    /// [`Cx::spawn_bg_with`] for work that answers no one.
    pub fn spawn_bg_detached_with<Fut, F>(&self, work: F)
    where
        Fut: Future<Output = ()> + 'static + Send,
        F: FnOnce(Cancel) -> Fut,
    {
        self.bg_detached(work(self.addr.cancellation()), true);
    }

    fn bg_detached<Fut>(&self, fut: Fut, listens: bool)
    where
        Fut: Future<Output = ()> + 'static + Send,
    {
        let cancel = self.addr.cancellation();
        let task = Task::new::<A>(self.addr.id, "()");
        let spawned = trace::mark(|| task.spawn());

        #[cfg(feature = "test-utils")]
        let counted = crate::actor::event_bus::Counted::new();

        crate::executor::spawn(async move {
            #[cfg(feature = "test-utils")]
            let _counted = counted;

            let running = trace::within(Some(spawned), fut);
            let ran = if listens {
                running.await;
                true
            } else {
                cancel.guard(running).await.is_some()
            };

            let ended = match ran && !cancel.is_cancelled() {
                true => task.settled(),
                false => task.cancelled(),
            };

            task.ended(spawned, ended);
        });
    }

    /// Feeds this actor what `source` yields, each item as `into(item)`, for
    /// as long as the actor lives: a watch, a subscription, a pipe that
    /// pushes.
    ///
    /// A source is not work of what opened it, as a timer is not work of
    /// what started it. The handler that opens it is done once it is open,
    /// and each item arrives as a root of its own - so an action that opens
    /// a watch is finished when the watch is open, not when it runs dry.
    ///
    /// Declared as `bg` in `actor!`, like any other work that answers later.
    ///
    /// ```ignore
    /// cx.spawn_source(changes, Changed);
    /// ```
    pub fn spawn_source<S, Out, F>(&self, source: S, into: F)
    where
        S: Stream + Send + 'static,
        Out: Send + 'static,
        F: FnMut(S::Item) -> Out + Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: crate::actor::flow::Allows<M, Out>,
    {
        let id = self.addr.id;
        let cancel = self.addr.cancellation();
        let feed = Feed::new::<A>(id, short_type_name::<Out>());
        let opened = trace::mark(|| feed.opened());

        crate::executor::spawn(async move {
            let pouring = trace::within(None, pour::<A, _, _, _>(id, source, into, feed, opened));
            let ran_dry = cancel.guard(pouring).await.is_some();

            feed.closed(opened, !ran_dry || cancel.is_cancelled());
        });
    }
}

/// Hands each item of `source` to the UI thread, as a root of its own, until
/// the source runs dry. Gives way after each item, so a source that is always
/// ready still notices its actor going.
async fn pour<A, S, Out, F>(id: usize, source: S, mut into: F, feed: Feed, opened: Cause)
where
    A: Handler<Out> + 'static,
    S: Stream,
    Out: Send + 'static,
    F: FnMut(S::Item) -> Out,
{
    let mut source = std::pin::pin!(source);

    loop {
        let Some(item) = std::future::poll_fn(|cx| source.as_mut().poll_next(cx)).await else {
            break;
        };
        let message = into(item);

        invoke_on_ui(move || {
            let _root = trace::resume(None);
            let _arrived = trace::enter(|| feed.arrived(opened));

            if let Some(addr) = registered::<A>(id) {
                addr.send(message);
            }
        });

        crate::executor::yield_now().await;
    }
}

pub struct AsyncContext<A: 'static> {
    actor_id: usize,
    cancel: Cancel,
    _phantom: PhantomData<A>,
}

impl<A: 'static> Clone for AsyncContext<A> {
    fn clone(&self) -> Self {
        Self {
            actor_id: self.actor_id,
            cancel: self.cancel.clone(),
            _phantom: PhantomData,
        }
    }
}

unsafe impl<A: 'static> Send for AsyncContext<A> {}
unsafe impl<A: 'static> Sync for AsyncContext<A> {}

impl<A: 'static> AsyncContext<A> {
    pub(crate) fn new(actor_id: usize, cancel: Cancel) -> Self {
        Self {
            actor_id,
            cancel,
            _phantom: PhantomData,
        }
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

    /// What `f` makes of the actor, on the UI thread - and `None` when there
    /// is no actor left to ask.
    ///
    /// Gone is an ordinary answer here, not a failure: background work
    /// outlives a teardown often enough, and the alternative was a panic on
    /// a thread nobody is watching.
    pub async fn apply<R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut A, &Cx<A>) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let id = self.actor_id;
        let cause = trace::current();

        invoke_on_ui(move || {
            let _resumed = trace::resume(cause);
            if let Some(addr) = registered::<A>(id) {
                addr.apply(move |actor, ctx| {
                    let result = f(actor, ctx);
                    let _ = tx.send(result);
                });
            }
        });

        rx.await.ok()
    }

    pub fn send<M>(&self, msg: M)
    where
        M: Send + 'static,
        A: Handler<M>,
    {
        let id = self.actor_id;
        let cause = trace::current();

        invoke_on_ui(move || {
            if let Some(addr) = registered::<A>(id) {
                addr.send_under(msg, cause);
            }
        });
    }
}

impl<A: 'static, M> Cx<A, M> {
    pub fn async_ctx(&self) -> AsyncContext<A> {
        AsyncContext::new(self.addr.id, self.addr.cancellation())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::UiThreadToken;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct First;
    struct Second;

    struct Chain {
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    guinea_macros::actor! {
        Chain {
            handlers {
                First => { send Second, bg Second }
                Second
            }
        }
    }

    impl Handler<First> for Chain {
        fn handle(&mut self, _: First, cx: Cx<Self, First>) {
            self.log.borrow_mut().push("first");
            cx.send(Second);
        }
    }

    impl Handler<Second> for Chain {
        fn handle(&mut self, _: Second, _cx: Cx<Self, Second>) {
            self.log.borrow_mut().push("second");
        }
    }

    #[test]
    fn send_from_a_handler_is_drained_by_the_same_queue() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new_scoped(
            Chain { log: log.clone() },
            UiThreadToken::dangerously_create_token_unchecked(),
        );

        addr.send(First);

        assert_eq!(&*log.borrow(), &["first", "second"]);
    }

    #[test]
    fn a_chain_of_sends_is_traced_back_to_the_action_that_started_it() {
        use crate::trace::{Cause, Record, Trace};

        let seen = Rc::new(RefCell::new(Vec::<Record>::new()));
        let sink = seen.clone();
        trace::observe(move |trace| {
            if let Trace::Begin(record) | Trace::Mark(record) = trace {
                sink.borrow_mut().push(record.clone());
            }
        });

        let addr = Addr::new_scoped(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            UiThreadToken::dangerously_create_token_unchecked(),
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
        let addr = Addr::new_scoped(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            UiThreadToken::dangerously_create_token_unchecked(),
        );

        let cx = Cx::<_, First>::new(addr.clone());
        let cancel = cx.cancellation();
        assert!(!cancel.is_cancelled());

        addr.dispose();

        assert!(cancel.is_cancelled(), "teardown has to reach the tasks too");
    }

    #[tokio::test]
    async fn a_task_cancelled_with_its_actor_says_so_and_answers_nobody() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |trace| {
            if let crate::trace::Trace::Mark(record) = trace {
                sink.borrow_mut().push(record.point.kind());
            }
        });

        let log = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new_managed_scoped(
            Chain { log: log.clone() },
            UiThreadToken::dangerously_create_token_unchecked(),
        );

        let cx = Cx::<_, First>::new(addr.clone());
        cx.spawn_bg::<Second, _>(async {
            std::future::pending::<()>().await;
            Second
        });

        addr.dispose();
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        trace::stop_observing();

        let seen = seen.borrow();
        assert_eq!(*seen, ["spawn", "cancelled"], "the task's whole life");
        assert!(log.borrow().is_empty(), "nothing came back to be handled");
    }

    #[tokio::test]
    async fn work_that_listens_for_the_token_is_left_to_wind_itself_down() {
        let addr = Addr::new_managed_scoped(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            UiThreadToken::dangerously_create_token_unchecked(),
        );

        let wound_down = Arc::new(AtomicBool::new(false));
        let noted = wound_down.clone();

        let cx = Cx::<_, First>::new(addr.clone());
        cx.spawn_bg_with::<Second, _, _>(|gone| async move {
            gone.cancelled().await;
            tokio::task::yield_now().await;
            noted.store(true, Ordering::SeqCst);
            Second
        });

        addr.dispose();
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        assert!(
            wound_down.load(Ordering::SeqCst),
            "the await after the cancellation still ran"
        );
    }

    #[test]
    fn detach_keeps_the_address() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new_scoped(
            Chain { log },
            UiThreadToken::dangerously_create_token_unchecked(),
        );

        let cx = Cx::<_, First>::new(addr.clone());
        let bare = cx.detach();

        assert_eq!(bare.addr().id(), addr.id());
    }
}
