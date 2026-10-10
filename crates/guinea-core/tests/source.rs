//! A source that is always ready never gives way by itself: the actor going
//! away has to stop it all the same.
//!
//! A process of its own, because the dispatcher is process-wide.

use futures_core::Stream;
use guinea_core::actor::{
    Addr, Cx, Handler, Home, UiDispatcher, UiTask, UiThreadToken, set_ui_dispatcher,
};
use guinea_core::scope::ScopeTree;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

struct Forever(Arc<AtomicUsize>);

impl Stream for Forever {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<()>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(()))
    }
}

struct Open(Forever);

struct Pulled;

#[derive(Debug)]
struct Puller;

guinea_macros::actor! {
    Puller {
        handlers {
            Open => { bg Pulled }
            Pulled
        }
    }
}

impl Handler<Open> for Puller {
    fn handle(&mut self, Open(source): Open, cx: Cx<Self, Open>) {
        cx.spawn_source(source, |()| Pulled);
    }
}

impl Handler<Pulled> for Puller {
    fn handle(&mut self, _: Pulled, _cx: Cx<Self, Pulled>) {}
}

struct Nowhere;

impl UiDispatcher for Nowhere {
    fn init(&self) {}

    fn dispatch(&self, _: UiTask) {}
}

#[test]
fn a_source_that_never_waits_still_stops_with_its_actor() {
    set_ui_dispatcher(Nowhere);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let entered = runtime.enter();

    let pulled = Arc::new(AtomicUsize::new(0));
    let tree = ScopeTree::new();
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let addr = Addr::new_managed(Puller, &Home::new(tree.scope(), token, None));
    addr.send(Open(Forever(pulled.clone())));

    let deadline = Instant::now() + Duration::from_secs(5);
    while pulled.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = pulled.load(Ordering::SeqCst) > 0;

    addr.dispose();
    std::thread::sleep(Duration::from_millis(200));
    let stopped = pulled.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    let later = pulled.load(Ordering::SeqCst);

    drop(entered);
    runtime.shutdown_background();

    assert!(started, "the source was running");
    assert_eq!(later, stopped, "nothing is pulled once the actor is gone");
}
