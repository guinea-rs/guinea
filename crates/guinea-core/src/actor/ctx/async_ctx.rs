use super::Cx;
use crate::actor::addr::Reach;
use crate::actor::cancel::Cancel;
use crate::actor::event_bus::subscribe::Event;
use crate::actor::event_bus::{EventBus, RpcCall};
use crate::actor::traits::Handler;
use crate::actor::{AllowedSignal, ManagedActor};
use crate::trace;
use crate::ui_thread::invoke_on_ui;
use std::time::Duration;

/// What work off the UI thread keeps of the actor that started it, from
/// [`Cx::async_ctx`]: `Send`, where the handler's [`Cx`] is not.
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
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// let cancel = ctx.cancellation();
    ///
    /// job.dispose();
    /// assert!(cancel.is_cancelled());
    /// ```
    pub fn cancellation(&self) -> Cancel {
        self.cancel.clone()
    }

    /// Whether the actor is gone. Worth asking between steps of long work:
    /// what comes after is for nobody.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// assert!(!ctx.is_cancelled());
    ///
    /// job.dispose();
    /// assert!(ctx.is_cancelled());
    /// ```
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Waits until the actor is gone - one arm of a `select!`, for work that
    /// has to end itself rather than be dropped at an await.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// use std::time::Duration;
    ///
    /// let runtime = tokio::runtime::Runtime::new().unwrap();
    /// let waiting = runtime.spawn(async move { ctx.cancelled().await });
    /// assert!(!waiting.is_finished(), "waits while the actor is there");
    ///
    /// job.dispose();
    /// let woken = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(5), waiting).await });
    /// assert!(woken.is_ok(), "woken by the actor going");
    /// ```
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    /// `fut`'s output, or `None` if the actor went away first.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// let runtime = tokio::runtime::Runtime::new().unwrap();
    /// assert_eq!(runtime.block_on(ctx.until_gone(async { 2 })), Some(2));
    ///
    /// job.dispose();
    /// assert_eq!(runtime.block_on(ctx.until_gone(async { 2 })), None);
    /// ```
    pub async fn until_gone<F: Future>(&self, fut: F) -> Option<F::Output> {
        self.cancel.guard(fut).await
    }

    /// Publishes `msg` on the bus of the actor's tree, from whichever thread
    /// this runs on: the bus lives on the UI thread, and the event goes there
    /// with its cause.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # guinea_macros::actor! {
    /// #     Job {
    /// #         handlers { Begin }
    /// #         publishes { Saved }
    /// #     }
    /// # }
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::Event;
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// #[derive(Clone)]
    /// struct Saved;
    ///
    /// impl Event for Saved {}
    ///
    /// let ui = UiLoop::install();
    /// let heard_on = Rc::new(Cell::new(None));
    /// let heard = heard_on.clone();
    /// let _subscription =
    ///     tree.bus().subscribe_fn(move |_: Saved| heard.set(Some(std::thread::current().id())));
    ///
    /// std::thread::spawn(move || ctx.publish(Saved)).join().unwrap();
    /// assert_eq!(heard_on.get(), None, "not heard where it was published");
    ///
    /// ui.run();
    /// assert_eq!(heard_on.get(), Some(std::thread::current().id()), "heard on the UI thread");
    /// ```
    pub fn publish<M>(&self, msg: M)
    where
        A: ManagedActor,
        M: Event,
        A::Signals: AllowedSignal<M>,
    {
        self.reach.scope().publish(msg);
    }

    /// Publishes `msg` on `bus`, a window's: on the UI thread, where the bus
    /// lives.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # guinea_macros::actor! {
    /// #     Job {
    /// #         handlers { Begin }
    /// #         publishes { Saved }
    /// #     }
    /// # }
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::event_bus::{Event, EventBus};
    ///
    /// #[derive(Clone)]
    /// struct Saved;
    ///
    /// impl Event for Saved {}
    ///
    /// let window = Rc::new(EventBus::new());
    /// let saved = Rc::new(Cell::new(false));
    /// let heard = saved.clone();
    /// let _subscription = window.subscribe_fn(move |_: Saved| heard.set(true));
    ///
    /// ctx.publish_local(&window, Saved);
    /// assert!(saved.get());
    /// ```
    pub fn publish_local<M>(&self, bus: &EventBus, msg: M)
    where
        A: ManagedActor,
        M: Event,
        A::Signals: AllowedSignal<M>,
    {
        bus.publish(msg);
    }

    /// Asks `payload` on the bus of the actor's tree and waits for its one
    /// answer, from whichever thread this runs on.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # struct Job(Rc<RefCell<Option<AsyncContext<Job>>>>);
    /// # struct Begin;
    /// # impl Handler<Begin> for Job {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.0.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// # let tree = ScopeTree::new();
    /// # let slot = Rc::new(RefCell::new(None));
    /// # let job = Addr::new(Job(slot.clone()), &Home::new(tree.scope(), None));
    /// # job.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    /// use std::time::Duration;
    ///
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// #[derive(Clone, guinea_macros::Request)]
    /// #[request(reply = Doubled)]
    /// struct Double(u32);
    ///
    /// #[derive(Clone)]
    /// struct Doubled(u32);
    ///
    /// let ui = UiLoop::install();
    /// let _answers = tree.bus().answer_fn(|Double(n)| Doubled(n * 2));
    ///
    /// let asking = std::thread::spawn(move || {
    ///     let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    ///     runtime.block_on(ctx.request(Double(21), Duration::from_secs(5)))
    /// });
    ///
    /// assert!(ui.run_until(|| asking.is_finished(), Duration::from_secs(5)));
    /// assert_eq!(asking.join().unwrap().unwrap().0, 42);
    /// ```
    pub async fn request<Req: RpcCall>(
        &self,
        payload: Req,
        timeout: Duration,
    ) -> anyhow::Result<Req::Response> {
        self.reach.scope().request(payload, timeout).await
    }

    /// Sends `msg` to the actor from whichever thread this runs on, with its
    /// cause: it is handled on the UI thread, and not at all once the actor
    /// is gone.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use guinea_core::actor::{AsyncContext, Cx, Handler};
    /// # struct Begin;
    /// # impl Handler<Begin> for Total {
    /// #     fn handle(&mut self, _: Begin, cx: Cx<Self, Begin>) {
    /// #         *self.1.borrow_mut() = Some(cx.async_ctx());
    /// #     }
    /// # }
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use guinea_core::actor::{Addr, Home};
    /// use guinea_core::scope::ScopeTree;
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// struct Total(Rc<Cell<u32>>, Rc<RefCell<Option<AsyncContext<Total>>>>);
    ///
    /// struct Add(u32);
    ///
    /// impl Handler<Add> for Total {
    ///     fn handle(&mut self, Add(n): Add, _cx: Cx<Self, Add>) {
    ///         self.0.set(self.0.get() + n);
    ///     }
    /// }
    ///
    /// let ui = UiLoop::install();
    /// let tree = ScopeTree::new();
    /// let total = Rc::new(Cell::new(0));
    /// # let slot = Rc::new(RefCell::new(None));
    /// let addr = Addr::new(Total(total.clone(), slot.clone()), &Home::new(tree.scope(), None));
    /// # addr.send(Begin);
    /// # let ctx = slot.borrow_mut().take().unwrap();
    ///
    /// let later = ctx.clone();
    /// std::thread::spawn(move || ctx.send(Add(2))).join().unwrap();
    /// assert_eq!(total.get(), 0, "not handled on the thread that sent it");
    ///
    /// ui.run();
    /// assert_eq!(total.get(), 2, "handled on the UI thread");
    ///
    /// addr.dispose();
    /// std::thread::spawn(move || later.send(Add(3))).join().unwrap();
    /// ui.run();
    /// assert_eq!(total.get(), 2, "nobody to handle it");
    /// ```
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
