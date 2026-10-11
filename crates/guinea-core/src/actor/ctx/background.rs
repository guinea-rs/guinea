use super::Cx;
use crate::actor::cancel::Cancel;
use crate::actor::flow::Allows;
use crate::actor::traits::Handler;
use crate::actor::ManagedActor;
use crate::short_type_name;
use crate::trace::{self, Cause, Point};
use crate::ui_thread::invoke_on_ui;
use std::time::Instant;

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
    /// polled it last: [`crate::observability::mark_anywhere`] carries it to
    /// the thread being watched.
    fn ended(self, spawned: Cause, point: Point) {
        let _resumed = trace::resume(Some(spawned));

        crate::observability::mark_anywhere(move || point);
    }
}

impl<A: 'static, M> Cx<A, M> {
    /// Runs `fut` off the UI thread and sends what it returns back to this
    /// actor.
    ///
    /// The task lives as long as the actor: once the actor's scope goes, `fut`
    /// is dropped where it last awaited and nothing is sent.
    pub fn spawn_bg<Out, Fut>(&self, fut: Fut)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
        Fut: Future<Output = Out> + 'static + Send,
    {
        self.bg(fut, false);
    }

    /// [`Cx::spawn_bg`] for work that wants to hear the cancellation
    /// rather than be dropped by it: a loop that checks between rounds, a
    /// connection that says goodbye, a `select!` arm.
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    /// # use std::time::Duration;
    /// # use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # use guinea_core::ui_thread::UiLoop;
    /// # #[derive(Debug)]
    /// # struct Poller { polls: Arc<AtomicUsize>, stopped: Arc<AtomicBool> }
    /// # struct Start;
    /// # struct Tick;
    /// # guinea_macros::actor! { Poller { handlers { Start => { bg Tick } Tick } } }
    /// # impl Handler<Tick> for Poller {
    /// #     fn handle(&mut self, _: Tick, _cx: Cx<Self, Tick>) {}
    /// # }
    /// # impl Handler<Start> for Poller {
    /// #     fn handle(&mut self, _: Start, cx: Cx<Self, Start>) {
    /// #         let (polls, stopped) = (self.polls.clone(), self.stopped.clone());
    /// #         let poll = move || {
    /// #             let polls = polls.clone();
    /// #             async move {
    /// #                 tokio::time::sleep(Duration::from_millis(1)).await;
    /// #                 polls.fetch_add(1, Ordering::SeqCst);
    /// #             }
    /// #         };
    /// cx.spawn_bg_with::<Tick, _, _>(|gone| async move {
    ///     while !gone.is_cancelled() {
    ///         poll().await;
    ///     }
    ///     # stopped.store(true, Ordering::SeqCst);
    ///     Tick
    /// });
    /// #     }
    /// # }
    /// # let ui = UiLoop::install();
    /// # let runtime = tokio::runtime::Runtime::new().unwrap();
    /// # let _entered = runtime.enter();
    /// # let tree = ScopeTree::new();
    /// # let polls = Arc::new(AtomicUsize::new(0));
    /// # let stopped = Arc::new(AtomicBool::new(false));
    /// # let poller = Poller { polls: polls.clone(), stopped: stopped.clone() };
    /// # Addr::new(poller, &Home::new(tree.scope())).send(Start);
    /// # assert!(ui.run_until(|| polls.load(Ordering::SeqCst) >= 3, Duration::from_secs(5)));
    /// # assert!(!stopped.load(Ordering::SeqCst), "it polls while the actor lives");
    /// # tree.remove();
    /// # assert!(ui.run_until(|| stopped.load(Ordering::SeqCst), Duration::from_secs(5)), "and stops when it goes");
    /// ```
    pub fn spawn_bg_with<Out, Fut, F>(&self, work: F)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
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

    /// [`Cx::spawn_bg`], with `listens` for work that was handed the
    /// token and winds itself down: it is left to finish instead of being
    /// dropped at its next await. Either way nothing is sent to an actor
    /// that is gone.
    ///
    /// What the work returns comes back to the actor as a message:
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    /// use std::time::Duration;
    ///
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// #[derive(Debug)]
    /// struct Weather(Rc<Cell<Option<i32>>>);
    ///
    /// struct Fetch;
    ///
    /// struct Fetched(i32);
    ///
    /// guinea_macros::actor! {
    ///     Weather {
    ///         handlers {
    ///             Fetch => { bg Fetched }
    ///             Fetched
    ///         }
    ///     }
    /// }
    ///
    /// impl Handler<Fetch> for Weather {
    ///     fn handle(&mut self, _: Fetch, cx: Cx<Self, Fetch>) {
    ///         cx.spawn_bg(async { Fetched(21) });
    ///     }
    /// }
    ///
    /// impl Handler<Fetched> for Weather {
    ///     fn handle(&mut self, Fetched(degrees): Fetched, _cx: Cx<Self, Fetched>) {
    ///         self.0.set(Some(degrees));
    ///     }
    /// }
    ///
    /// let ui = UiLoop::install();
    /// let runtime = tokio::runtime::Runtime::new().unwrap();
    /// let _entered = runtime.enter();
    ///
    /// let tree = ScopeTree::new();
    /// let shown = Rc::new(Cell::new(None));
    /// let weather = Addr::new(Weather(shown.clone()), &Home::new(tree.scope()));
    ///
    /// weather.send(Fetch);
    /// assert_eq!(shown.get(), None, "the answer comes later, on the UI thread");
    ///
    /// assert!(ui.run_until(|| shown.get().is_some(), Duration::from_secs(5)));
    /// assert_eq!(shown.get(), Some(21));
    /// ```
    ///
    /// Work that listens for the token runs on after the actor is gone, to
    /// wind itself down; what it returns then is for nobody:
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    /// use std::time::Duration;
    ///
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// #[derive(Debug)]
    /// struct Weather(Rc<Cell<Option<i32>>>);
    ///
    /// struct Fetch(Arc<AtomicBool>);
    ///
    /// struct Fetched(i32);
    ///
    /// guinea_macros::actor! {
    ///     Weather {
    ///         handlers {
    ///             Fetch => { bg Fetched }
    ///             Fetched
    ///         }
    ///     }
    /// }
    ///
    /// impl Handler<Fetch> for Weather {
    ///     fn handle(&mut self, Fetch(closed): Fetch, cx: Cx<Self, Fetch>) {
    ///         cx.spawn_bg_with(|gone| async move {
    ///             gone.cancelled().await;
    ///             closed.store(true, Ordering::SeqCst);
    ///             Fetched(0)
    ///         });
    ///     }
    /// }
    ///
    /// impl Handler<Fetched> for Weather {
    ///     fn handle(&mut self, Fetched(degrees): Fetched, _cx: Cx<Self, Fetched>) {
    ///         self.0.set(Some(degrees));
    ///     }
    /// }
    ///
    /// let ui = UiLoop::install();
    /// let runtime = tokio::runtime::Runtime::new().unwrap();
    /// let _entered = runtime.enter();
    ///
    /// let tree = ScopeTree::new();
    /// let shown = Rc::new(Cell::new(None));
    /// let weather = Addr::new(Weather(shown.clone()), &Home::new(tree.scope()));
    /// let closed = Arc::new(AtomicBool::new(false));
    ///
    /// weather.send(Fetch(closed.clone()));
    /// tree.remove();
    ///
    /// assert!(ui.run_until(|| closed.load(Ordering::SeqCst), Duration::from_secs(5)), "it wound down");
    /// assert!(!ui.run_until(|| shown.get().is_some(), Duration::from_millis(100)), "and answered nobody");
    /// ```
    fn bg<Out, Fut>(&self, fut: Fut, listens: bool)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
        Fut: Future<Output = Out> + 'static + Send,
    {
        let reach = self.addr.reach();
        let cancel = self.addr.cancellation();
        let task = Task::new::<A>(reach.id(), short_type_name::<Out>());
        let spawned = trace::mark(|| task.spawn());

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
                let settled = trace::mark_under(Some(spawned), || task.settled());

                if let Some(addr) = reach.addr() {
                    addr.send_under(result, Some(settled));
                }
            };

            invoke_on_ui(return_task);
        });
    }

    fn bg_detached<Fut>(&self, fut: Fut, listens: bool)
    where
        Fut: Future<Output = ()> + 'static + Send,
    {
        let cancel = self.addr.cancellation();
        let task = Task::new::<A>(self.addr.id(), "()");
        let spawned = trace::mark(|| task.spawn());

        crate::executor::spawn(async move {
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
}

#[cfg(test)]
mod tests {
    use super::super::chain::{Chain, First, Second};
    use super::*;
    use crate::actor::addr::{Addr, TestHome};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn a_task_cancelled_with_its_actor_says_so_and_answers_nobody() {
        let home = TestHome::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        trace::observe(move |trace| {
            if let crate::trace::Trace::Mark(record) = trace {
                sink.borrow_mut().push(record.point.kind());
            }
        });

        let log = Rc::new(RefCell::new(Vec::new()));
        let addr = Addr::new(
            Chain { log: log.clone() },
            &home,
        );

        let cx = Cx::<_, First>::new(addr.clone());
        cx.spawn_bg::<Second, _>(async {
            std::future::pending::<()>().await;
            Second
        });

        home.tree.remove();
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
        let home = TestHome::new();
        let addr = Addr::new(
            Chain {
                log: Rc::new(RefCell::new(Vec::new())),
            },
            &home,
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

        home.tree.remove();
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        assert!(
            wound_down.load(Ordering::SeqCst),
            "the await after the cancellation still ran"
        );
    }
}
