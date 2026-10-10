//! An async handler publishes from a runtime worker, the way the application
//! runs it: the event has to reach the UI thread's global bus, not the
//! worker's own empty one.
//!
//! A process of its own, because the dispatcher is process-wide.

use guinea_core::actor::event_bus::GlobalEventBus;
use guinea_core::actor::{
    Addr, AsyncContext, Cx, Handler, Home, UiDispatcher, UiTask, UiThreadToken,
    set_ui_dispatcher,
};
use guinea_core::scope::ScopeTree;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, guinea_macros::Event)]
struct Refreshed(u32);

struct Refresh;

struct Refresher {
    handed: Arc<Mutex<Option<AsyncContext<Refresher>>>>,
}

impl std::fmt::Debug for Refresher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Refresher")
    }
}

guinea_macros::actor! {
    Refresher {
        handlers { Refresh }
        publishes { Refreshed }
    }
}

impl Handler<Refresh> for Refresher {
    fn handle(&mut self, _: Refresh, cx: Cx<Self, Refresh>) {
        *self.handed.lock().unwrap() = Some(cx.async_ctx());
    }
}

/// The UI thread is the test's own: what is dispatched comes back to it
/// through a channel.
struct Channel(Mutex<mpsc::Sender<UiTask>>);

impl UiDispatcher for Channel {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        let _ = self.0.lock().unwrap().send(task);
    }
}

#[test]
fn an_event_published_off_the_ui_thread_reaches_the_ui_thread_s_subscribers() {
    let (to_ui, ui) = mpsc::channel();
    set_ui_dispatcher(Channel(Mutex::new(to_ui)));

    let heard = Rc::new(RefCell::new(Vec::new()));
    let hearing = heard.clone();
    let _subscription = GlobalEventBus::subscribe_fn(move |Refreshed(n): Refreshed| {
        hearing.borrow_mut().push(n);
    });

    let handed: Arc<Mutex<Option<AsyncContext<Refresher>>>> = Arc::default();
    let tree = ScopeTree::new();
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let addr = Addr::new_managed(
        Refresher {
            handed: handed.clone(),
        },
        &Home::new(tree.scope(), token, None),
    );
    addr.send(Refresh);
    let ctx = handed.lock().unwrap().take().expect("the handler ran on this thread");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        tokio::spawn(async move { ctx.publish(Refreshed(7)) })
            .await
            .unwrap();
    });

    let publish = ui
        .recv_timeout(Duration::from_secs(5))
        .expect("the publication was handed to the UI thread");
    publish();

    assert_eq!(*heard.borrow(), [7]);
}
