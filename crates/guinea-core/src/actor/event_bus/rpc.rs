use crate::actor::event_bus::{Answering, Event};
use crate::actor::shape::Declared;
use crate::actor::traits::Handler;
use crate::actor::In;
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

type Waiting<R> = Arc<Mutex<Option<oneshot::Sender<Result<R, Why>>>>>;

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

    fn settle(&self, outcome: Result<R, Why>) -> bool {
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
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a request, so nothing waits for what its handler returns",
    label = "not a request",
    note = "a handler returns a value only to answer a request, `#[derive(guinea::Request)]` with `#[request(reply = ..)]`; a handler for a message returns nothing"
)]
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
/// # use guinea_core::__private::Home;
/// # use guinea_core::actor::{Addr, In};
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
/// #     Actions { subscribes { RpcRequest<Act> } }
/// # }
/// #[handler]
/// fn act(In(this, Act(action), _): In<Actions, Act>) -> Reply<Outcome> {
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

    fn handle_rpc(handed: In<'_, Self, Req>) -> Reply<Req::Response>
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

    fn handle(In(this, msg, cx): In<Self, RpcRequest<Req>>) {
        let (payload, reply_to) = msg.into_parts();
        A::handle_rpc(In(this, payload, cx.asked())).send(reply_to);
    }
}

impl Scope {
    /// Asks `payload` on this scope's [`bus`](Scope::bus) and waits for the
    /// one answer, from whichever thread this runs on: the request is
    /// published on the UI thread, and fails at once when nothing awake
    /// answers it.
    pub async fn request<Req>(self, payload: Req, timeout: Duration) -> Result<Req::Response, Unanswered>
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

        let carried = trace::carry();
        invoke_on_ui(move || {
            let _carried = carried.resume();

            let why = match self.tree_bus() {
                None => Why::TreeGone,
                Some(bus) => match bus.answering::<RpcRequest<Req>>() {
                    Answering::Awake => return bus.publish(envelope),
                    Answering::Nobody => Why::Nobody,
                    Answering::Asleep(answerer) => Why::Asleep { answerer },
                },
            };

            envelope.reply_to.settle(Err(why));
        });

        let why = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(response))) => return Ok(response),
            Ok(Ok(Err(why))) => why,
            Ok(Err(_)) => Why::Dropped,
            Err(_) => Why::TimedOut,
        };

        Err(Unanswered {
            request: std::any::type_name::<Req>(),
            why,
        })
    }
}

/// Why a request came back without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unanswered {
    /// The request, as its type is named.
    pub request: &'static str,
    pub why: Why,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The tree it was asked in is gone.
    TreeGone,
    /// Nothing in the tree answers it.
    Nobody,
    /// What answers it lives in a scope that is asleep.
    Asleep { answerer: &'static str },
    /// What answers it let go of it without answering.
    Dropped,
    /// No answer came in the time it was given.
    TimedOut,
}

impl std::fmt::Display for Unanswered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let request = self.request;
        match self.why {
            Why::TreeGone => write!(f, "{request} was asked in a tree that is gone"),
            Why::Nobody => write!(f, "nobody answers {request}"),
            Why::Asleep { answerer } => write!(f, "{answerer}, which answers {request}, is asleep"),
            Why::Dropped => write!(f, "{request} was dropped without an answer"),
            Why::TimedOut => write!(f, "{request} was not answered in time"),
        }
    }
}

impl std::error::Error for Unanswered {}
