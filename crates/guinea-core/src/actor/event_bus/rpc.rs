use crate::actor::event_bus::{Answering, Event};
use crate::actor::shape::Declared;
use crate::actor::traits::Handler;
use crate::actor::Cx;
use crate::scope::Scope;
use crate::trace;
use crate::ui_thread::invoke_on_ui;
use parking_lot::Mutex;
use std::any::TypeId;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

tokio::task_local! {
    /// The chain of `RpcCall` types whose replies are currently pending
    /// somewhere up this task's ancestry, oldest first. Set by
    /// `ReplyTo::later` around a handler body when it starts running,
    /// so that any `Scope::request` called from within that body - even
    /// transitively, through further handlers answering `later` on other
    /// actors - sees it. See `Scope::request`'s cycle check below for why
    /// this exists.
    static RPC_CHAIN: Vec<TypeId>;
}

fn current_rpc_chain() -> Vec<TypeId> {
    RPC_CHAIN.try_with(|c| c.clone()).unwrap_or_default()
}

/// A request on its way to what answers it, carrying where the answer goes.
#[derive(Clone)]
pub struct RpcRequest<T: RpcCall> {
    pub payload: T,
    reply_to: ReplyTo<T::Response>,
}

impl<T: RpcCall> Event for RpcRequest<T> {
    /// Without the way back: a listener hears the request, and a reply from
    /// it lands nowhere.
    fn overheard(self) -> Self {
        Self {
            reply_to: self.reply_to.nowhere(),
            ..self
        }
    }
}

type Waiting<R> = Arc<Mutex<Option<oneshot::Sender<Result<R, String>>>>>;

/// Where the answer to one request goes: to whoever awaits it, once.
///
/// The request carries it, so nothing outside the request keeps track of
/// what is pending. When every copy that could answer is dropped unanswered,
/// the one who asked hears so at once rather than at its timeout.
pub struct ReplyTo<R> {
    waiting: Option<Waiting<R>>,
    /// See `RPC_CHAIN`: the requests already awaiting answers up this call
    /// chain, carried into [`ReplyTo::later`].
    chain: Vec<TypeId>,
}

impl<R> Clone for ReplyTo<R> {
    fn clone(&self) -> Self {
        Self {
            waiting: self.waiting.clone(),
            chain: self.chain.clone(),
        }
    }
}

impl<R: Send + 'static> ReplyTo<R> {
    fn nowhere(self) -> Self {
        Self {
            waiting: None,
            ..self
        }
    }

    fn settle(&self, outcome: Result<R, String>) -> bool {
        let Some(waiting) = &self.waiting else {
            return false;
        };

        if let Some(tx) = waiting.lock().take() {
            let _ = tx.send(outcome);
        }
        true
    }

    /// Answers with `value`. From something that only hears the request it
    /// goes nowhere.
    pub fn send(self, value: R) {
        if !self.settle(Ok(value)) {
            tracing::warn!(
                reply = std::any::type_name::<R>(),
                "a reply from something that only hears the request went nowhere"
            );
        }
    }

    /// Answers with what `work` comes to, run in the background with this
    /// request's chain - so a request made from inside it, even through
    /// other actors, is caught when it closes a cycle.
    ///
    /// Not cut short when the actor that answers is disposed: somebody is
    /// waiting for a value, and a dropped future would leave them only the
    /// timeout. A handler that would rather stop early has the token through
    /// its `AsyncContext` - but it still owes an answer.
    pub fn later<Fut>(self, work: Fut)
    where
        Fut: Future<Output = R> + Send + 'static,
    {
        let chain = self.chain.clone();
        crate::executor::spawn(RPC_CHAIN.scope(chain, async move {
            self.send(work.await);
        }));
    }
}

/// A request on a tree's bus, and what answers it. Written with
/// `#[derive(guinea::Request)]` and `#[request(reply = Type)]`.
///
/// One subscriber answers it - a handler that returns `Response`, or
/// [`EventBus::answer_fn`](super::EventBus::answer_fn). A handler that
/// returns nothing only hears it; a `Handler<RpcRequest<Req>>` written by hand
/// answers only if it says `const ANSWERS: bool = true`.
pub trait RpcCall: Clone + Send + 'static {
    type Response: Clone + Send + 'static;
}

impl<Req: RpcCall> RpcRequest<Req> {
    /// Answers it now. See [`ReplyTo::send`].
    pub fn reply(self, response: Req::Response) {
        self.reply_to.send(response);
    }

    /// What was asked, and where the answer goes - for a handler that answers
    /// after it is done with the request.
    pub fn into_parts(self) -> (Req, ReplyTo<Req::Response>) {
        (self.payload, self.reply_to)
    }
}

/// What a request is answered with: a value now, or the value work in the
/// background comes to.
///
/// A handler that answers later reads what it needs from its actor first,
/// on the UI thread, and hands the rest to the background:
///
/// ```
/// # use std::time::Duration;
/// # use guinea_core::actor::{Addr, Home};
/// # use guinea_core::actor::event_bus::{Reply, RpcRequest};
/// # use guinea_core::scope::ScopeTree;
/// # use guinea_core::ui_thread::UiLoop;
/// # use guinea_macros::handler;
/// # #[derive(Clone, Debug, guinea_macros::Request)]
/// # #[request(reply = Outcome)]
/// # struct Act(u32);
/// # #[derive(Clone, Debug, PartialEq)]
/// # enum Outcome { Done(u32), NotConnected }
/// # #[derive(Clone, Debug)]
/// # struct Transport;
/// # impl Transport {
/// #     async fn act(&self, action: u32) -> Outcome { Outcome::Done(action) }
/// # }
/// # #[derive(Debug)]
/// # struct Actions { transport: Option<Transport> }
/// # guinea_macros::actor! {
/// #     Actions { handlers { RpcRequest<Act> } subscribes { RpcRequest<Act> } }
/// # }
/// #[handler]
/// fn act(this: &mut Actions, Act(action): Act) -> Reply<Outcome> {
///     let Some(transport) = this.transport.clone() else {
///         return Reply::now(Outcome::NotConnected);
///     };
///     Reply::later(async move { transport.act(action).await })
/// }
/// # let ui = UiLoop::install();
/// # let runtime = tokio::runtime::Runtime::new().unwrap();
/// # let _entered = runtime.enter();
/// # let ask = |transport: Option<Transport>| {
/// #     let tree = ScopeTree::new();
/// #     Addr::new(Actions { transport }, &Home::new(tree.scope()));
/// #     let asked = runtime.spawn(tree.scope().request::<Act>(Act(7), Duration::from_secs(5)));
/// #     assert!(ui.run_until(|| asked.is_finished(), Duration::from_secs(5)));
/// #     runtime.block_on(asked).unwrap().unwrap()
/// # };
/// # assert_eq!(ask(Some(Transport)), Outcome::Done(7));
/// # assert_eq!(ask(None), Outcome::NotConnected);
/// ```
///
/// A handler that returns the reply itself answers now; `Reply` is for
/// the one that may not.
pub struct Reply<T>(Answer<T>);

enum Answer<T> {
    Now(T),
    Later(std::pin::Pin<Box<dyn Future<Output = T> + Send>>),
}

impl<T> Reply<T> {
    pub fn now(value: T) -> Self {
        Self(Answer::Now(value))
    }

    /// Answers with what `work` comes to, run in the background. It is not
    /// cut short when the actor goes: somebody is waiting for the value.
    pub fn later(work: impl Future<Output = T> + Send + 'static) -> Self {
        Self(Answer::Later(Box::pin(work)))
    }
}

impl<T> From<T> for Reply<T> {
    fn from(value: T) -> Self {
        Self::now(value)
    }
}

impl<T: Send + 'static> Reply<T> {
    fn send(self, to: ReplyTo<T>) {
        match self.0 {
            Answer::Now(value) => to.send(value),
            Answer::Later(work) => to.later(work),
        }
    }
}

/// The type-level half of the request/response contract: `handle_rpc`
/// returns the reply instead of taking a `RpcRequest<Req>` and being
/// trusted to call `.reply(...)` somewhere inside its body. There is no way
/// to compile a `RpcHandler` impl that forgets to reply, replies twice, or
/// replies with the wrong type - the blanket `Handler<RpcRequest<Req>>`
/// impl below is the only thing that replies, exactly once, with what
/// `handle_rpc` returned: at once, or when its [`Reply::later`] is done.
pub trait RpcHandler<Req: RpcCall>: 'static {
    /// Where the handler was written; `#[handler]` fills it in.
    const DECLARED: Option<Declared> = None;

    fn handle_rpc(&mut self, req: Req, cx: Cx<Self, Req>) -> Reply<Req::Response>
    where
        Self: Sized;
}

impl<A, Req> Handler<RpcRequest<Req>> for A
where
    A: RpcHandler<Req> + 'static,
    Req: RpcCall,
{
    const DECLARED: Option<Declared> = <A as RpcHandler<Req>>::DECLARED;

    const ANSWERS: bool = true;

    fn handle(&mut self, msg: RpcRequest<Req>, cx: Cx<Self, RpcRequest<Req>>) {
        let (payload, reply_to) = msg.into_parts();
        self.handle_rpc(payload, cx.handling()).send(reply_to);
    }
}

impl Scope {
    /// Asks `payload` on this scope's [`bus`](Scope::bus) and waits for the
    /// one answer, from whichever thread this runs on: the request is
    /// published on the UI thread, and fails at once when nothing awake
    /// answers it.
    pub async fn request<Req>(self, payload: Req, timeout: Duration) -> anyhow::Result<Req::Response>
    where
        Req: RpcCall,
    {
        let req_type = TypeId::of::<Req>();
        let chain = current_rpc_chain();

        // A cycle here means some handler up this call chain is - directly
        // or transitively, via other actors - awaiting a reply that can
        // only ever be produced after *this* request resolves. That can
        // never happen; every hop is stuck waiting on the next one forever.
        // Letting it run to `timeout` would just make the failure slow and
        // its cause invisible (a generic "RPC request timed out" many
        // layers away from the actual cycle) - panicking immediately, with
        // the chain that proves it, turns a silent multi-actor wedge into
        // an obvious bug report at the exact call site that closed the loop.
        if chain.contains(&req_type) {
            panic!(
                "RPC cycle detected requesting {} - it (or a request that led back \
                 to it) is already awaiting its own reply {} level(s) up this call chain. This \
                 can never resolve: each hop is waiting on the next, all the way back to itself.",
                std::any::type_name::<Req>(),
                chain.len(),
            );
        }

        let mut next_chain = chain;
        next_chain.push(req_type);

        let (tx, rx) = oneshot::channel();
        let envelope = RpcRequest {
            payload,
            reply_to: ReplyTo {
                waiting: Some(Arc::new(Mutex::new(Some(tx)))),
                chain: next_chain,
            },
        };

        let cause = trace::current();
        invoke_on_ui(move || {
            let _resumed = trace::resume(cause);

            let unanswered = match self.tree_bus() {
                None => "the tree it was asked in is gone".to_string(),
                Some(bus) => match bus.answering::<RpcRequest<Req>>() {
                    Answering::Awake => return bus.publish(envelope),
                    Answering::Nobody => format!("nobody answers {}", std::any::type_name::<Req>()),
                    Answering::Asleep(answerer) => format!(
                        "{answerer}, which answers {}, is asleep",
                        std::any::type_name::<Req>()
                    ),
                },
            };

            envelope.reply_to.settle(Err(unanswered));
        });

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(unanswered))) => Err(anyhow::anyhow!(unanswered)),
            Ok(Err(_)) => Err(anyhow::anyhow!(
                "{} was dropped without an answer",
                std::any::type_name::<Req>()
            )),
            Err(_) => Err(anyhow::anyhow!("RPC request timed out")),
        }
    }
}

#[cfg(all(test, feature = "test-utils"))]
mod tests {
    use super::*;
    use crate::actor::event_bus::EventBus;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration as StdDuration;

    /// Every test here drives the bus by hand via `crate::ui_thread::run_queued()`,
    /// and under `test-utils` `invoke_on_ui` funnels all work into one
    /// process-wide queue. A drain is therefore not scoped to the
    /// test that issued it: run two of these concurrently (cargo's default)
    /// and one test consumes the other's queued `RpcRequest` deliveries, so
    /// the rightful owner never sees its reply and dies on
    /// `RPC request timed out`. Serializing the module with a plain `Mutex` held for each
    /// test's duration is enough; `into_inner` on poisoning keeps a single
    /// failing test from cascading into bogus failures everywhere else.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn request_reply_round_trip_same_thread() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Pong)]
        struct Ping;
        #[derive(Clone, Debug)]
        struct Pong;

        let tree = crate::scope::ScopeTree::new();
        let _sub = tree.bus().answer_fn(|_: Ping| Pong);

        let handle = tokio::spawn(tree.scope().request::<Ping>(Ping, StdDuration::from_secs(1)));
        // Let the spawned task run up to its `rx.await` - `request` queues
        // the publish synchronously before that point, so one yield is
        // enough for the queued task to exist by the time we drain it.
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued();

        assert!(handle.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn rpc_handler_replies_exactly_once_via_the_blanket_impl() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::Addr;
        use crate::actor::addr::TestHome;

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Echoed)]
        struct Echo(u32);
        #[derive(Clone, Debug)]
        struct Echoed(u32);

        struct EchoActor;
        impl RpcHandler<Echo> for EchoActor {
            fn handle_rpc(&mut self, Echo(n): Echo, _cx: Cx<Self, Echo>) -> Reply<Echoed> {
                Reply::now(Echoed(n * 2))
            }
        }

        let home = TestHome::new();
        let addr = Addr::bare(EchoActor, &home);
        let _sub = home.tree.bus().subscribe::<EchoActor, RpcRequest<Echo>>(addr);

        let handle =
            tokio::spawn(home.tree.scope().request::<Echo>(Echo(21), StdDuration::from_secs(1)));
        tokio::task::yield_now().await;
        // One drain publishes `RpcRequest<Echo>`, a second lets the actor
        // handle it and reply.
        crate::ui_thread::run_queued();
        crate::ui_thread::run_queued();

        let response = handle.await.unwrap().expect("rpc handler should have replied");
        assert_eq!(response.0, 42);
    }

    #[tokio::test]
    async fn a_request_nobody_answers_fails_at_once() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = NeverReplied)]
        struct Unasked;
        #[derive(Clone, Debug)]
        struct NeverReplied;

        let tree = crate::scope::ScopeTree::new();
        let _hears = tree.bus().subscribe_fn(|_: RpcRequest<Unasked>| {});

        let handle =
            tokio::spawn(tree.scope().request::<Unasked>(Unasked, StdDuration::from_secs(60)));
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued();

        let error = tokio::time::timeout(StdDuration::from_secs(5), handle)
            .await
            .expect("it waited for a reply nothing could give")
            .unwrap()
            .expect_err("nothing answers it");
        assert!(error.to_string().contains("nobody answers"), "{error}");
    }

    #[test]
    #[should_panic(expected = "a request has exactly one answerer")]
    fn a_second_answerer_is_refused_when_it_subscribes() {
        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Answer)]
        struct Question;
        #[derive(Clone, Debug)]
        struct Answer;

        let bus = Rc::new(EventBus::new());
        let _first = bus.answer_fn(|_: Question| Answer);
        let _second = bus.answer_fn(|_: Question| Answer);
    }

    #[test]
    #[should_panic(expected = "a request has exactly one answerer")]
    fn two_actors_returning_the_reply_cannot_both_answer() {
        use crate::actor::Addr;
        use crate::actor::addr::TestHome;

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Answer)]
        struct Question;
        #[derive(Clone, Debug)]
        struct Answer;

        struct Service;
        struct Monitor;
        impl RpcHandler<Question> for Service {
            fn handle_rpc(&mut self, _: Question, _cx: Cx<Self, Question>) -> Reply<Answer> {
                Reply::now(Answer)
            }
        }
        impl RpcHandler<Question> for Monitor {
            fn handle_rpc(&mut self, _: Question, _cx: Cx<Self, Question>) -> Reply<Answer> {
                Reply::now(Answer)
            }
        }

        let home = TestHome::new();
        let bus = Rc::new(EventBus::new());
        let _service = bus.subscribe::<Service, RpcRequest<Question>>(Addr::bare(Service, &home));
        let _monitor = bus.subscribe::<Monitor, RpcRequest<Question>>(Addr::bare(Monitor, &home));
    }

    #[test]
    fn the_answerer_s_place_is_free_again_once_it_goes() {
        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Answer)]
        struct Question;
        #[derive(Clone, Debug)]
        struct Answer;

        let bus = Rc::new(EventBus::new());
        drop(bus.answer_fn(|_: Question| Answer));
        let _next = bus.answer_fn(|_: Question| Answer);
    }

    #[tokio::test]
    async fn a_listener_hears_the_request_and_its_reply_goes_nowhere() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Count)]
        struct HowMany;
        #[derive(Clone, Debug, PartialEq)]
        struct Count(u32);

        let tree = crate::scope::ScopeTree::new();
        let heard = Arc::new(AtomicBool::new(false));
        let hearing = heard.clone();
        let _listener = tree.bus().subscribe_fn(move |request: RpcRequest<HowMany>| {
            hearing.store(true, Ordering::SeqCst);
            request.reply(Count(0));
        });
        let _answerer = tree.bus().answer_fn(|_: HowMany| Count(7));

        let handle =
            tokio::spawn(tree.scope().request::<HowMany>(HowMany, StdDuration::from_secs(1)));
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued();
        crate::ui_thread::run_queued();

        assert_eq!(handle.await.unwrap().unwrap(), Count(7));
        assert!(heard.load(Ordering::SeqCst), "the listener was not told");
    }

    #[tokio::test]
    async fn a_request_whose_answerer_sleeps_fails_at_once() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::Addr;
        use crate::actor::addr::TestHome;

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Done)]
        struct Work;
        #[derive(Clone, Debug)]
        struct Done;

        struct Worker;
        impl RpcHandler<Work> for Worker {
            fn handle_rpc(&mut self, _: Work, _cx: Cx<Self, Work>) -> Reply<Done> {
                Reply::now(Done)
            }
        }

        let home = TestHome::new();
        let addr = Addr::bare(Worker, &home);
        let _sub = home.tree.bus().subscribe::<Worker, RpcRequest<Work>>(addr);
        home.tree.sleep();

        let handle =
            tokio::spawn(home.tree.scope().request::<Work>(Work, StdDuration::from_secs(60)));
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued();

        let error = tokio::time::timeout(StdDuration::from_secs(5), handle)
            .await
            .expect("it waited for an answerer that sleeps")
            .unwrap()
            .expect_err("its answerer sleeps");
        assert!(error.to_string().contains("is asleep"), "{error}");
    }

    #[tokio::test]
    async fn a_request_its_answerer_drops_fails_at_once() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::Addr;
        use crate::actor::addr::TestHome;

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Done)]
        struct Work;
        #[derive(Clone, Debug)]
        struct Done;

        struct Dropper;
        impl Handler<RpcRequest<Work>> for Dropper {
            const ANSWERS: bool = true;

            fn handle(&mut self, _: RpcRequest<Work>, _cx: Cx<Self, RpcRequest<Work>>) {}
        }

        let home = TestHome::new();
        let addr = Addr::bare(Dropper, &home);
        let _sub = home.tree.bus().subscribe::<Dropper, RpcRequest<Work>>(addr);

        let handle =
            tokio::spawn(home.tree.scope().request::<Work>(Work, StdDuration::from_secs(60)));
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued();
        crate::ui_thread::run_queued();

        let outcome = tokio::time::timeout(StdDuration::from_secs(5), handle).await;
        let said = match &outcome {
            Ok(Ok(Err(error))) => error.to_string(),
            _ => String::new(),
        };
        assert!(said.contains("without an answer"), "{outcome:?}");
    }

    /// A requester awaiting on one OS thread reaches an answerer on another:
    /// the normal shape, answerer on the UI thread and requester on a
    /// `spawn_bg` worker. The request is published where the tree is.
    #[test]
    fn request_resolves_when_subscriber_is_on_a_different_os_thread() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Pong)]
        struct Ping;
        #[derive(Clone, Debug)]
        struct Pong;

        let (ready_tx, ready_rx) = std_mpsc::channel::<Scope>();
        let (stop_tx, stop_rx) = std_mpsc::channel::<()>();

        let ui_thread = std::thread::spawn(move || {
            let tree = crate::scope::ScopeTree::new();
            let _sub = tree.bus().answer_fn(|_: Ping| Pong);
            ready_tx.send(tree.scope()).unwrap();
            while stop_rx.try_recv().is_err() {
                crate::ui_thread::run_queued();
                std::thread::sleep(StdDuration::from_millis(5));
            }
        });

        let scope = ready_rx.recv().unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let result = rt.block_on(scope.request::<Ping>(Ping, StdDuration::from_secs(2)));

        stop_tx.send(()).unwrap();
        ui_thread.join().unwrap();

        assert!(
            result.is_ok(),
            "expected a reply delivered from another OS thread, got {result:?}"
        );
    }

    /// Exercises the `#[handler]` macro's RPC heuristic end to end for both
    /// its sync and async branches (see guinea-macros/src/handler.rs) - not
    /// just the hand-written `RpcHandler` impl the other tests use.
    #[tokio::test]
    async fn handler_macro_rpc_heuristic_sync_and_async() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::addr::TestHome;
        use crate::actor::{Addr, AsyncContext};
        use guinea_macros::handler;

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Doubled)]
        struct Double(u32);
        #[derive(Clone, Debug)]
        struct Doubled(u32);

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Sum)]
        struct DelayedAdd(u32, u32);
        #[derive(Clone, Debug)]
        struct Sum(u32);

        struct MathActor;

        // Sync branch: a plain return type, no `.reply()` anywhere in sight.
        #[handler]
        fn double(this: &mut MathActor, Double(n): Double) -> Doubled {
            let _ = this;
            Doubled(n * 2)
        }

        // Async branch: `.await`s before producing the value the macro
        // replies with.
        #[handler]
        async fn delayed_add(_ctx: AsyncContext<MathActor>, req: DelayedAdd) -> Sum {
            tokio::time::sleep(StdDuration::from_millis(1)).await;
            Sum(req.0 + req.1)
        }

        let home = TestHome::new();
        let addr = Addr::bare(MathActor, &home);
        let bus = home.tree.bus();
        let scope = home.tree.scope();
        let _sub_double = bus.subscribe::<MathActor, RpcRequest<Double>>(addr.clone());
        let _sub_add = bus.subscribe::<MathActor, RpcRequest<DelayedAdd>>(addr.clone());

        let double_handle =
            tokio::spawn(scope.request::<Double>(Double(21), StdDuration::from_secs(1)));
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued();
        crate::ui_thread::run_queued();
        assert_eq!(double_handle.await.unwrap().unwrap().0, 42);

        let add_handle = tokio::spawn(
            scope.request::<DelayedAdd>(DelayedAdd(2, 3), StdDuration::from_secs(1)),
        );
        tokio::task::yield_now().await;
        crate::ui_thread::run_queued(); // deliver RpcRequest<DelayedAdd> -> spawns the async body
        // The async body needs actual wall-clock time (`tokio::time::sleep`)
        // before it replies, so poll the queue a few times instead of
        // draining once.
        for _ in 0..20 {
            tokio::time::sleep(StdDuration::from_millis(5)).await;
            crate::ui_thread::run_queued();
        }
        assert_eq!(add_handle.await.unwrap().unwrap().0, 5);
    }

    /// The scenario the cycle check exists for: actor A's async RPC handler
    /// calls out to actor B, and actor B's handler calls back into A for
    /// the *same request type* A is still waiting on - a genuine cross-actor
    /// deadlock (A can't reply until B replies, B can't reply until A's
    /// original request resolves). Without the `RPC_CHAIN` check in
    /// `Scope::request`, this would just sit until both requests' 5s
    /// timeouts expired. With it, the request for `ReqA` inside B's
    /// handler must panic immediately, because `ReplyTo::later` carried the
    /// in-flight chain `[ReqA, ReqB]` across the `tokio::spawn` boundary
    /// into B's handler body.
    ///
    /// Panics inside a `tokio::spawn`ed task don't propagate to this test
    /// function's own thread - tokio catches them into a `JoinError` on a
    /// `JoinHandle` nobody here holds. A process-wide panic hook is the only
    /// way to observe that the panic happened at all, which makes this test
    /// unsafe to run concurrently with anything else that installs its own
    /// hook - run this module with `--test-threads=1` (already required by
    /// the rest of it, which shares the task queue).
    #[tokio::test]
    async fn cross_actor_rpc_cycle_is_detected_immediately() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::addr::TestHome;
        use crate::actor::{Addr, AsyncContext};
        use guinea_macros::handler;
        use std::panic;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = RespA)]
        struct ReqA(u32);
        #[derive(Clone, Debug)]
        struct RespA(u32);

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = RespB)]
        struct ReqB(u32);
        #[derive(Clone, Debug)]
        struct RespB(u32);

        struct ActorA;
        struct ActorB;

        #[handler]
        async fn handle_req_a(ctx: AsyncContext<ActorA>, req: ReqA) -> RespA {
            let RespB(n) = ctx
                .request::<ReqB>(ReqB(req.0), StdDuration::from_secs(5))
                .await
                .expect("should never resolve normally - the cycle panics first");
            RespA(n)
        }

        #[handler]
        async fn handle_req_b(ctx: AsyncContext<ActorB>, req: ReqB) -> RespB {
            // Closes the loop: same `ReqA` type is already on the chain.
            let RespA(n) = ctx
                .request::<ReqA>(ReqA(req.0), StdDuration::from_secs(5))
                .await
                .expect("should never resolve normally - the cycle panics first");
            RespB(n)
        }

        // Every panic, not the first. The hook is process-wide and the test
        // binary is one process: another test panicking on purpose - and one
        // does, to poison a lock - would otherwise take this slot and this
        // test would fail reporting someone else's message.
        let panics: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let hook_slot = panics.clone();
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            hook_slot.lock().unwrap().push(info.to_string());
        }));

        let seen_the_cycle = || {
            panics
                .lock()
                .unwrap()
                .iter()
                .any(|message| message.contains("RPC cycle detected"))
        };

        let home = TestHome::new();
        let addr_a = Addr::bare(ActorA, &home);
        let addr_b = Addr::bare(ActorB, &home);
        let _sub_a = home.tree.bus().subscribe::<ActorA, RpcRequest<ReqA>>(addr_a);
        let _sub_b = home.tree.bus().subscribe::<ActorB, RpcRequest<ReqB>>(addr_b);

        let handle =
            tokio::spawn(home.tree.scope().request::<ReqA>(ReqA(1), StdDuration::from_secs(5)));

        // Bounded, not open-ended - a regression where the cycle silently
        // stops being detected must fail this test quickly, not hang it.
        // Bounded by wall clock rather than by a count of scheduler turns:
        // 200 turns is a budget that shrinks with machine load, and this test
        // shares a process (and the task queue) with every other one.
        let deadline = std::time::Instant::now() + StdDuration::from_secs(5);
        while std::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
            crate::ui_thread::run_queued();
            if seen_the_cycle() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(1)).await;
        }

        panic::set_hook(prev_hook);
        handle.abort(); // the top-level request never gets a reply; don't wait out its timeout

        assert!(
            seen_the_cycle(),
            "expected the cycle check to panic inside actor B's handler; \
             the panics seen were: {:?}",
            panics.lock().unwrap()
        );
    }

    /// Stress test guarding against a self-deadlock: fires many concurrent
    /// `Scope::request` calls, from several OS threads at once, against a
    /// single `RpcHandler` actor pumped from yet another OS thread. Nothing
    /// here holds a lock across an `.await` (the lock on a request's
    /// `ReplyTo` is always a temporary, dropped before anything yields), so
    /// this should always complete - but that invariant is easy
    /// to break by accident in a future edit, and a broken invariant here
    /// means the whole app wedges the next time two RPCs race. If it ever
    /// does deadlock, the test must not just hang forever (which would burn
    /// CI time until an external timeout kills the runner with no useful
    /// message) - the watchdog thread below turns that hang into a fast,
    /// loud, obviously-a-deadlock failure instead.
    #[test]
    fn many_concurrent_requests_do_not_deadlock() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::Addr;
        use crate::actor::addr::TestHome;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone, Debug, guinea_macros::Request)]
        #[request(reply = Sum)]
        struct Add(u32, u32);
        #[derive(Clone, Debug)]
        struct Sum(u32);

        struct AddActor;
        impl RpcHandler<Add> for AddActor {
            fn handle_rpc(&mut self, Add(a, b): Add, _cx: Cx<Self, Add>) -> Reply<Sum> {
                Reply::now(Sum(a + b))
            }
        }

        let done = Arc::new(AtomicBool::new(false));
        let watchdog_done = done.clone();
        let watchdog = std::thread::spawn(move || {
            for _ in 0..100 {
                if watchdog_done.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(StdDuration::from_millis(50));
            }
            eprintln!(
                "many_concurrent_requests_do_not_deadlock: deadline exceeded \
                 without completing - treating this as a deadlock and \
                 aborting instead of hanging"
            );
            std::process::abort();
        });

        let (ready_tx, ready_rx) = std_mpsc::channel::<Scope>();
        let (stop_tx, stop_rx) = std_mpsc::channel::<()>();

        // Stands in for the UI thread, same shape as the cross-thread test
        // above: owns the actor, subscribes it, and pumps the dispatcher
        // queue on an interval.
        let ui_thread = std::thread::spawn(move || {
            let home = TestHome::new();
            let addr = Addr::bare(AddActor, &home);
            let _sub = home.tree.bus().subscribe::<AddActor, RpcRequest<Add>>(addr);
            ready_tx.send(home.tree.scope()).unwrap();
            while stop_rx.try_recv().is_err() {
                crate::ui_thread::run_queued();
                std::thread::sleep(StdDuration::from_millis(2));
            }
        });

        let scope = ready_rx.recv().unwrap();

        const REQUESTER_THREADS: u32 = 8;
        const REQUESTS_PER_THREAD: u32 = 25;

        let requesters: Vec<_> = (0..REQUESTER_THREADS)
            .map(|t| {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .unwrap();
                    rt.block_on(async {
                        for i in 0..REQUESTS_PER_THREAD {
                            let result = scope
                                .request::<Add>(Add(t, i), StdDuration::from_secs(5))
                                .await
                                    .unwrap_or_else(|e| panic!("request {t}/{i} failed: {e}"));
                            assert_eq!(result.0, t + i);
                        }
                    });
                })
            })
            .collect();

        for r in requesters {
            r.join().unwrap();
        }

        stop_tx.send(()).unwrap();
        ui_thread.join().unwrap();

        // Only reached if every request/reply round trip above actually
        // completed - tell the watchdog it can stand down.
        done.store(true, Ordering::SeqCst);
        watchdog.join().unwrap();
    }
}
