use crate::actor::Cx;
use crate::actor::event_bus::GlobalEventBus;
use crate::actor::event_bus::Event;
use crate::actor::traits::Handler;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;
use tokio::sync::oneshot;
use uuid::Uuid;

tokio::task_local! {
    /// The chain of `RpcCall` types whose replies are currently pending
    /// somewhere up this task's ancestry, oldest first. Set by
    /// `AsyncBus::spawn_reply` around a handler body when it starts running,
    /// so that any `AsyncBus::request` called from within that body - even
    /// transitively, through further `spawn_reply`d handlers on other
    /// actors - sees it. See `AsyncBus::request`'s cycle check below for why
    /// this exists.
    static RPC_CHAIN: Vec<TypeId>;
}

fn current_rpc_chain() -> Vec<TypeId> {
    RPC_CHAIN.try_with(|c| c.clone()).unwrap_or_default()
}

#[derive(Clone)]
pub struct RpcRequest<T> {
    pub correlation_id: Uuid,
    pub payload: T,
    /// See `RPC_CHAIN`. Only meaningful to `AsyncBus::spawn_reply` - a
    /// hand-written `Handler<RpcRequest<Req>>` that spawns its own reply
    /// task without going through `spawn_reply` simply won't get cycle
    /// detection for requests made from inside it.
    pub chain: Vec<TypeId>,
}

impl<T: RpcCall> Event for RpcRequest<T> {}

#[derive(Clone)]
pub struct RpcResponse<T> {
    pub correlation_id: Uuid,
    pub payload: T,
}

impl<T: Clone + Send + 'static> Event for RpcResponse<T> {}

pub trait RpcCall: Clone + Send + 'static {
    type Response: Clone + Send + 'static;
}

impl<Req: RpcCall> RpcRequest<Req> {
    pub fn reply(self, response: Req::Response) {
        AsyncBus::reply(self.correlation_id, response);
    }
}

/// The type-level half of the request/response contract: `handle_rpc`
/// returns `Req::Response` directly instead of taking a `RpcRequest<Req>`
/// and being trusted to call `.reply(...)` somewhere inside its body.
/// There is no way to compile a `RpcHandler` impl that forgets to reply,
/// replies twice, or replies with the wrong type - the blanket
/// `Handler<RpcRequest<Req>>` impl below is the only caller of
/// `AsyncBus::reply`, and it always calls it exactly once, after
/// `handle_rpc` returns.
///
/// Implement this instead of `Handler<RpcRequest<Req>>` directly whenever
/// the reply is available synchronously from within the handler. If the
/// reply depends on async work (an RPC to a remote agent, `spawn_bg`,
/// etc.), implement `Handler<RpcRequest<Req>>` by hand instead and call
/// `msg.reply(...)` once the async work completes - the two are mutually
/// exclusive for a given `Req` (implementing both would conflict on the
/// same `Handler<RpcRequest<Req>>` impl).
pub trait RpcHandler<Req: RpcCall>: 'static {
    /// Where the handler was written; `#[handler]` fills it in.
    const DECLARED: Option<crate::actor::shape::Declared> = None;

    fn handle_rpc(&mut self, req: Req, cx: Cx<Self, Req>) -> Req::Response
    where
        Self: Sized;
}

impl<A, Req> Handler<RpcRequest<Req>> for A
where
    A: RpcHandler<Req> + 'static,
    Req: RpcCall,
{
    const DECLARED: Option<crate::actor::shape::Declared> = <A as RpcHandler<Req>>::DECLARED;

    fn handle(&mut self, msg: RpcRequest<Req>, cx: Cx<Self, RpcRequest<Req>>) {
        let response = self.handle_rpc(msg.payload, cx.handling());
        AsyncBus::reply(msg.correlation_id, response);
    }
}

static PENDING_REQUESTS: Lazy<RwLock<HashMap<Uuid, oneshot::Sender<Box<dyn Any + Send>>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub struct AsyncBus;

impl AsyncBus {
    pub async fn request<Req>(payload: Req, timeout: Duration) -> anyhow::Result<Req::Response>
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
                "AsyncBus: RPC cycle detected requesting {} - it (or a request that led back \
                 to it) is already awaiting its own reply {} level(s) up this call chain. This \
                 can never resolve: each hop is waiting on the next, all the way back to itself.",
                std::any::type_name::<Req>(),
                chain.len(),
            );
        }

        let mut next_chain = chain;
        next_chain.push(req_type);

        let correlation_id = Uuid::new_v4();
        let envelope = RpcRequest {
            correlation_id,
            payload,
            chain: next_chain,
        };

        let (tx, rx) = oneshot::channel();
        PENDING_REQUESTS.write().insert(correlation_id, tx);

        // Must go through the UI-thread dispatcher, not
        // `GlobalEventBus::instance().publish(...)` directly - the bus is
        // thread_local, and `request` is typically awaited from a
        // `spawn_bg` future running on a background tokio thread. Publishing
        // there would hit an empty, subscriber-less bus instance and always
        // time out.
        GlobalEventBus::publish(envelope);

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(any_res)) => match any_res.downcast::<RpcResponse<Req::Response>>() {
                Ok(res) => Ok(res.payload),
                Err(_) => Err(anyhow::anyhow!("Type mismatch in async response")),
            },
            Ok(Err(_)) => Err(anyhow::anyhow!("Response channel closed")),
            Err(_) => {
                PENDING_REQUESTS.write().remove(&correlation_id);
                Err(anyhow::anyhow!("RPC request timed out"))
            }
        }
    }

    pub fn reply<Res>(correlation_id: Uuid, payload: Res)
    where
        Res: Clone + Send + 'static,
    {
        let envelope = RpcResponse {
            correlation_id,
            payload,
        };

        if let Some(tx) = PENDING_REQUESTS.write().remove(&correlation_id) {
            let _ = tx.send(Box::new(envelope.clone()));
        }

        // Same reasoning as in `request`: route through the UI-thread
        // dispatcher rather than the calling thread's own bus instance.
        GlobalEventBus::publish(envelope);
    }

    /// Runs `fut` to completion in a spawned task and replies to
    /// `correlation_id` with whatever it produces, with `fut` running
    /// inside `chain`'s `RPC_CHAIN` scope - so any `AsyncBus::request` made
    /// from within `fut` (even transitively, on some other actor `fut`
    /// itself makes a request to) is correctly attributed to this call
    /// chain for cycle detection.
    ///
    /// This is the async-RPC-handler counterpart to `RpcHandler`'s blanket
    /// sync impl: the only thing that calls `reply` here is this function,
    /// after `fut` resolves, so it still holds exactly once. It exists as a
    /// standalone entry point (rather than folded into a trait like
    /// `RpcHandler`) because `#[handler]`'s generated async-RPC glue and any
    /// hand-written `Handler<RpcRequest<Req>>` both need to call it the same
    /// way, and a trait can't express "produces a value after an await" any
    /// more precisely than `Future<Output = Req::Response>` already does.
    /// Unlike [`Cx::spawn_bg`], this is not cut short when the actor
    /// that answers is disposed: somebody is waiting for a value, and the
    /// only thing a dropped future would leave them is the request's
    /// timeout. A handler that would rather stop early has the token through
    /// its `AsyncContext` - but it still owes an answer.
    ///
    /// [`Cx::spawn_bg`]: crate::actor::Cx::spawn_bg
    pub fn spawn_reply<Res, Fut>(correlation_id: Uuid, chain: Vec<TypeId>, fut: Fut)
    where
        Res: Clone + Send + 'static,
        Fut: Future<Output = Res> + Send + 'static,
    {
        #[cfg(feature = "test-utils")]
        let counted = crate::actor::event_bus::Counted::new();

        crate::executor::spawn(RPC_CHAIN.scope(chain, async move {
            #[cfg(feature = "test-utils")]
            let _counted = counted;

            let response = fut.await;
            AsyncBus::reply(correlation_id, response);
        }));
    }
}

#[cfg(all(test, feature = "test-utils"))]
mod tests {
    use super::*;
    use crate::actor::event_bus::EventBus;
    use std::sync::Mutex;
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration as StdDuration;

    /// Every test here drives the bus by hand via `EventBus::process_queue()`,
    /// and under `test-utils` `invoke_on_ui` funnels all work into one
    /// process-wide `TEST_TASK_QUEUE`. A drain is therefore not scoped to the
    /// test that issued it: run two of these concurrently (cargo's default)
    /// and one test consumes the other's queued `RpcRequest`/`RpcResponse`
    /// deliveries, so the rightful owner never sees its reply and dies on
    /// `RPC request timed out`. `PENDING_REQUESTS` is global for the same
    /// reason. Serializing the module with a plain `Mutex` held for each
    /// test's duration is enough; `into_inner` on poisoning keeps a single
    /// failing test from cascading into bogus failures everywhere else.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn request_reply_round_trip_same_thread() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[derive(Clone, Debug)]
        struct Ping;
        #[derive(Clone, Debug)]
        struct Pong;
        crate::rpc_bind!(Ping => Pong);

        let _sub =
            GlobalEventBus::instance().subscribe_fn::<RpcRequest<Ping>>(|req| req.reply(Pong));

        let handle = tokio::spawn(AsyncBus::request::<Ping>(Ping, StdDuration::from_secs(1)));
        // Let the spawned task run up to its `rx.await` - `request` queues
        // the publish synchronously before that point, so one yield is
        // enough for the queued task to exist by the time we drain it.
        tokio::task::yield_now().await;
        EventBus::process_queue();
        // `reply()` also queues an `RpcResponse` broadcast - drain it too so
        // it doesn't linger in the global queue for a later test.
        EventBus::process_queue();

        assert!(handle.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn rpc_handler_replies_exactly_once_via_the_blanket_impl() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::{Addr, UiThreadToken};

        #[derive(Clone, Debug)]
        struct Echo(u32);
        #[derive(Clone, Debug)]
        struct Echoed(u32);
        crate::rpc_bind!(Echo => Echoed);

        struct EchoActor;
        impl RpcHandler<Echo> for EchoActor {
            fn handle_rpc(&mut self, Echo(n): Echo, _cx: Cx<Self, Echo>) -> Echoed {
                Echoed(n * 2)
            }
        }

        let addr = Addr::new_scoped(EchoActor, UiThreadToken::dangerously_create_token_unchecked());
        let _sub = GlobalEventBus::instance().subscribe::<EchoActor, RpcRequest<Echo>>(addr);

        let handle = tokio::spawn(AsyncBus::request::<Echo>(Echo(21), StdDuration::from_secs(1)));
        tokio::task::yield_now().await;
        // One drain delivers `RpcRequest<Echo>` to the actor (which replies
        // synchronously inside `handle`), a second drains the `RpcResponse`
        // broadcast that reply also queues.
        EventBus::process_queue();
        EventBus::process_queue();

        let response = handle.await.unwrap().expect("rpc handler should have replied");
        assert_eq!(response.0, 42);
    }

    #[tokio::test]
    async fn request_times_out_with_no_subscriber() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[derive(Clone, Debug)]
        struct Unanswered;
        #[derive(Clone, Debug)]
        struct NeverReplied;
        crate::rpc_bind!(Unanswered => NeverReplied);

        let result =
            AsyncBus::request::<Unanswered>(Unanswered, StdDuration::from_millis(50)).await;

        assert!(result.is_err());
    }

    /// Regression test for the bug fixed alongside this: `request`/`reply`
    /// used to call `GlobalEventBus::instance().publish(...)` directly,
    /// which resolves the thread_local bus of whichever OS thread happens
    /// to run that code. A requester awaiting from one OS thread and a
    /// subscriber registered on another (the normal shape - subscriber on
    /// the UI thread, requester on a `spawn_bg` worker) would silently miss
    /// each other and every request would time out. Going through
    /// `GlobalEventBus::publish`, which routes through the shared
    /// dispatcher queue, fixes that regardless of which thread each side
    /// runs on.
    #[test]
    fn request_resolves_when_subscriber_is_on_a_different_os_thread() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        #[derive(Clone, Debug)]
        struct Ping;
        #[derive(Clone, Debug)]
        struct Pong;
        crate::rpc_bind!(Ping => Pong);

        let (ready_tx, ready_rx) = std_mpsc::channel::<()>();
        let (stop_tx, stop_rx) = std_mpsc::channel::<()>();

        // Stands in for the real UI thread: owns its own thread_local
        // `EventBus` instance, subscribes there, and pumps the shared
        // dispatcher queue on an interval - the same shape a real
        // `UiDispatcher` runs in production.
        let ui_thread = std::thread::spawn(move || {
            let _sub = GlobalEventBus::instance()
                .subscribe_fn::<RpcRequest<Ping>>(|req| req.reply(Pong));
            ready_tx.send(()).unwrap();
            while stop_rx.try_recv().is_err() {
                EventBus::process_queue();
                std::thread::sleep(StdDuration::from_millis(5));
            }
        });

        ready_rx.recv().unwrap();

        // Deliberately a different OS thread than `ui_thread` above.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let result =
            rt.block_on(AsyncBus::request::<Ping>(Ping, StdDuration::from_secs(2)));

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
        use crate::actor::{Addr, AsyncContext, UiThreadToken};
        use guinea_macros::handler;

        #[derive(Clone, Debug)]
        struct Double(u32);
        #[derive(Clone, Debug)]
        struct Doubled(u32);
        crate::rpc_bind!(Double => Doubled);

        #[derive(Clone, Debug)]
        struct DelayedAdd(u32, u32);
        #[derive(Clone, Debug)]
        struct Sum(u32);
        crate::rpc_bind!(DelayedAdd => Sum);

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

        let addr =
            Addr::new_scoped(MathActor, UiThreadToken::dangerously_create_token_unchecked());
        let _sub_double =
            GlobalEventBus::instance().subscribe::<MathActor, RpcRequest<Double>>(addr.clone());
        let _sub_add = GlobalEventBus::instance()
            .subscribe::<MathActor, RpcRequest<DelayedAdd>>(addr.clone());

        let double_handle =
            tokio::spawn(AsyncBus::request::<Double>(Double(21), StdDuration::from_secs(1)));
        tokio::task::yield_now().await;
        EventBus::process_queue(); // deliver RpcRequest<Double> -> reply queued
        EventBus::process_queue(); // drain the RpcResponse broadcast
        assert_eq!(double_handle.await.unwrap().unwrap().0, 42);

        let add_handle = tokio::spawn(AsyncBus::request::<DelayedAdd>(
            DelayedAdd(2, 3),
            StdDuration::from_secs(1),
        ));
        tokio::task::yield_now().await;
        EventBus::process_queue(); // deliver RpcRequest<DelayedAdd> -> spawns the async body
        // The async body needs actual wall-clock time (`tokio::time::sleep`)
        // before it replies, so poll the queue a few times instead of
        // draining once.
        for _ in 0..20 {
            tokio::time::sleep(StdDuration::from_millis(5)).await;
            EventBus::process_queue();
        }
        assert_eq!(add_handle.await.unwrap().unwrap().0, 5);
    }

    /// The scenario the cycle check exists for: actor A's async RPC handler
    /// calls out to actor B, and actor B's handler calls back into A for
    /// the *same request type* A is still waiting on - a genuine cross-actor
    /// deadlock (A can't reply until B replies, B can't reply until A's
    /// original request resolves). Without the `RPC_CHAIN` check in
    /// `AsyncBus::request`, this would just sit until both requests' 5s
    /// timeouts expired. With it, `AsyncBus::request::<ReqA>` inside B's
    /// handler must panic immediately, because `spawn_reply` carried the
    /// in-flight chain `[ReqA, ReqB]` across the `tokio::spawn` boundary
    /// into B's handler body.
    ///
    /// Panics inside a `tokio::spawn`ed task don't propagate to this test
    /// function's own thread - tokio catches them into a `JoinError` on a
    /// `JoinHandle` nobody here holds. A process-wide panic hook is the only
    /// way to observe that the panic happened at all, which makes this test
    /// unsafe to run concurrently with anything else that installs its own
    /// hook - run this module with `--test-threads=1` (already required by
    /// the rest of it, which shares `GlobalEventBus`/`PENDING_REQUESTS`).
    #[tokio::test]
    async fn cross_actor_rpc_cycle_is_detected_immediately() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::{Addr, AsyncContext, UiThreadToken};
        use guinea_macros::handler;
        use std::panic;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Debug)]
        struct ReqA(u32);
        #[derive(Clone, Debug)]
        struct RespA(u32);
        crate::rpc_bind!(ReqA => RespA);

        #[derive(Clone, Debug)]
        struct ReqB(u32);
        #[derive(Clone, Debug)]
        struct RespB(u32);
        crate::rpc_bind!(ReqB => RespB);

        struct ActorA;
        struct ActorB;

        #[handler]
        async fn handle_req_a(_ctx: AsyncContext<ActorA>, req: ReqA) -> RespA {
            let RespB(n) = AsyncBus::request::<ReqB>(ReqB(req.0), StdDuration::from_secs(5))
                .await
                .expect("should never resolve normally - the cycle panics first");
            RespA(n)
        }

        #[handler]
        async fn handle_req_b(_ctx: AsyncContext<ActorB>, req: ReqB) -> RespB {
            // Closes the loop: same `ReqA` type is already on the chain.
            let RespA(n) = AsyncBus::request::<ReqA>(ReqA(req.0), StdDuration::from_secs(5))
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

        let addr_a = Addr::new_scoped(ActorA, UiThreadToken::dangerously_create_token_unchecked());
        let addr_b = Addr::new_scoped(ActorB, UiThreadToken::dangerously_create_token_unchecked());
        let _sub_a = GlobalEventBus::instance().subscribe::<ActorA, RpcRequest<ReqA>>(addr_a);
        let _sub_b = GlobalEventBus::instance().subscribe::<ActorB, RpcRequest<ReqB>>(addr_b);

        let handle = tokio::spawn(AsyncBus::request::<ReqA>(ReqA(1), StdDuration::from_secs(5)));

        // Bounded, not open-ended - a regression where the cycle silently
        // stops being detected must fail this test quickly, not hang it.
        // Bounded by wall clock rather than by a count of scheduler turns:
        // 200 turns is a budget that shrinks with machine load, and this test
        // shares a process (and the task queue) with every other one.
        let deadline = std::time::Instant::now() + StdDuration::from_secs(5);
        while std::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
            EventBus::process_queue();
            if seen_the_cycle() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(1)).await;
        }

        panic::set_hook(prev_hook);
        handle.abort(); // the top-level request never gets a reply; don't wait out its timeout

        assert!(
            seen_the_cycle(),
            "expected AsyncBus's cycle check to panic inside actor B's handler; \
             the panics seen were: {:?}",
            panics.lock().unwrap()
        );
    }

    /// Stress test guarding against a self-deadlock: fires many concurrent
    /// `AsyncBus::request` calls, from several OS threads at once, against a
    /// single `RpcHandler` actor pumped from yet another OS thread. Nothing
    /// here holds a lock across an `.await` (`PENDING_REQUESTS.write()` is
    /// always a temporary, dropped before either `request` or `reply`
    /// yields), so this should always complete - but that invariant is easy
    /// to break by accident in a future edit, and a broken invariant here
    /// means the whole app wedges the next time two RPCs race. If it ever
    /// does deadlock, the test must not just hang forever (which would burn
    /// CI time until an external timeout kills the runner with no useful
    /// message) - the watchdog thread below turns that hang into a fast,
    /// loud, obviously-a-deadlock failure instead.
    #[test]
    fn many_concurrent_requests_do_not_deadlock() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::actor::{Addr, UiThreadToken};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone, Debug)]
        struct Add(u32, u32);
        #[derive(Clone, Debug)]
        struct Sum(u32);
        crate::rpc_bind!(Add => Sum);

        struct AddActor;
        impl RpcHandler<Add> for AddActor {
            fn handle_rpc(&mut self, Add(a, b): Add, _cx: Cx<Self, Add>) -> Sum {
                Sum(a + b)
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

        let (ready_tx, ready_rx) = std_mpsc::channel::<()>();
        let (stop_tx, stop_rx) = std_mpsc::channel::<()>();

        // Stands in for the UI thread, same shape as the cross-thread test
        // above: owns the actor, subscribes it, and pumps the dispatcher
        // queue on an interval.
        let ui_thread = std::thread::spawn(move || {
            let addr =
                Addr::new_scoped(AddActor, UiThreadToken::dangerously_create_token_unchecked());
            let _sub = GlobalEventBus::instance().subscribe::<AddActor, RpcRequest<Add>>(addr);
            ready_tx.send(()).unwrap();
            while stop_rx.try_recv().is_err() {
                EventBus::process_queue();
                std::thread::sleep(StdDuration::from_millis(2));
            }
        });

        ready_rx.recv().unwrap();

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
                            let result =
                                AsyncBus::request::<Add>(Add(t, i), StdDuration::from_secs(5))
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

#[macro_export]
macro_rules! rpc_bind {
    ($( $req:ident => $res:ident );* $(;)?) => {
        $(
            impl $crate::actor::event_bus::rpc::RpcCall for $req {
                type Response = $res;
            }
        )*
    };
}
