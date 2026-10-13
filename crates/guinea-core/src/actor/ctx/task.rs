//! Background work a handler starts, and what becomes of new work while
//! older work of the same kind is still in flight.
//!
//! `cx.task()` starts it. By default every piece runs on its own and answers
//! when it is done, in whatever order that is. A policy puts it in a lane -
//! one per actor and type of answer, or per `.lane::<K>()` - and says what a
//! new piece does to the lane while it is busy:
//!
//! - [`latest`](Task::latest): the new piece replaces what runs, which is cut
//!   off and whose answer, if it comes anyway, is dropped.
//! - [`conflate`](Task::conflate): what runs finishes; the newest piece that
//!   came meanwhile runs next, and the ones between never start.
//! - [`exhaust`](Task::exhaust): nothing new is taken until the lane is free.
//!
//! A lane keeps the policy it was first given for as long as its actor lives:
//! two policies for one lane are two call sites that disagree, and the second
//! to spawn panics, naming both.

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::panic::Location;
use std::time::Instant;

use super::Cx;
use crate::actor::ManagedActor;
use crate::actor::addr::{Addr, Reach};
use crate::actor::cancel::{Cancel, Reason};
use crate::actor::flow::Allows;
use crate::actor::traits::Handler;
use crate::short_type_name;
use crate::trace::{self, Cause, Point};
use crate::ui_thread::{invoke_on_ui, try_invoke_on_ui};

/// Every piece runs on its own: the default.
pub struct Parallel;
/// A new piece replaces the one in flight.
pub struct Latest;
/// What is in flight finishes; then only the newest piece that came meanwhile.
pub struct Conflate;
/// Nothing new while a piece is in flight.
pub struct Exhaust;

/// What the work returns comes back to the actor as a message.
pub struct Reply;
/// The work answers no one.
pub struct Detached;

mod sealed {
    pub trait Sealed {}
}

/// What a lane does with new work while it is busy. See [the module](self).
pub trait Policy: sealed::Sealed {
    #[doc(hidden)]
    const KIND: Kind;
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Parallel,
    Latest,
    Conflate,
    Exhaust,
}

macro_rules! policy {
    ($($policy:ident),*) => {$(
        impl sealed::Sealed for $policy {}
        impl Policy for $policy {
            const KIND: Kind = Kind::$policy;
        }
    )*};
}

policy!(Parallel, Latest, Conflate, Exhaust);

/// Which lane: a type, and what it is called in a panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LaneId {
    type_id: TypeId,
    name: &'static str,
}

impl LaneId {
    fn of<K: 'static>() -> Self {
        Self {
            type_id: TypeId::of::<K>(),
            name: short_type_name::<K>(),
        }
    }
}

/// A key that splits a lane, compared by value.
pub(crate) trait LaneKey {
    fn same(&self, other: &dyn LaneKey) -> bool;
    fn as_any(&self) -> &dyn Any;
}

impl<K: Any + Eq> LaneKey for K {
    fn same(&self, other: &dyn LaneKey) -> bool {
        other.as_any().downcast_ref::<K>() == Some(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Background work about to be started; nothing runs until
/// [`spawn`](Task::spawn) or [`spawn_with`](Task::spawn_with).
#[must_use = "nothing starts until it is spawned"]
pub struct Task<'cx, A: 'static, M, P = Parallel, R = Reply> {
    cx: &'cx Cx<A, M>,
    lane: Option<LaneId>,
    key: Option<Box<dyn LaneKey>>,
    how: PhantomData<fn() -> (P, R)>,
}

impl<A: 'static, M> Cx<A, M> {
    /// Background work: run off the UI thread, with what it returns sent back
    /// to this actor. See [the module](crate::actor::ctx::task) for what a
    /// policy changes.
    ///
    /// It lives as long as the actor does: once the actor's scope goes, the
    /// work is dropped where it last awaited and nothing is sent.
    ///
    /// Without a policy every piece runs on its own and answers when it is
    /// done, in whatever order that is:
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::rc::Rc;
    /// use std::time::Duration;
    ///
    /// use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// use guinea_core::scope::ScopeTree;
    /// use guinea_core::ui_thread::UiLoop;
    ///
    /// struct Weather(Rc<RefCell<Vec<&'static str>>>);
    ///
    /// struct Fetch {
    ///     city: &'static str,
    ///     takes: Duration,
    /// }
    ///
    /// struct Fetched(&'static str);
    ///
    /// guinea_macros::actor! {
    ///     Weather {
    ///         handlers {
    ///             Fetch => { task Fetched }
    ///             Fetched
    ///         }
    ///     }
    /// }
    ///
    /// impl Handler<Fetch> for Weather {
    ///     fn handle(&mut self, Fetch { city, takes }: Fetch, cx: Cx<Self, Fetch>) {
    ///         cx.task().spawn(async move {
    ///             tokio::time::sleep(takes).await;
    ///             Fetched(city)
    ///         });
    ///     }
    /// }
    ///
    /// impl Handler<Fetched> for Weather {
    ///     fn handle(&mut self, Fetched(city): Fetched, _cx: Cx<Self, Fetched>) {
    ///         self.0.borrow_mut().push(city);
    ///     }
    /// }
    ///
    /// let ui = UiLoop::install();
    /// let runtime = tokio::runtime::Runtime::new().unwrap();
    /// let _entered = runtime.enter();
    ///
    /// let tree = ScopeTree::new();
    /// let shown = Rc::new(RefCell::new(Vec::new()));
    /// let weather = Addr::new(Weather(shown.clone()), &Home::new(tree.scope()));
    ///
    /// weather.send(Fetch { city: "Oslo", takes: Duration::from_millis(200) });
    /// weather.send(Fetch { city: "Rome", takes: Duration::ZERO });
    /// assert!(shown.borrow().is_empty(), "answers come later, on the UI thread");
    ///
    /// assert!(ui.run_until(|| shown.borrow().len() == 2, Duration::from_secs(5)));
    /// assert_eq!(*shown.borrow(), ["Rome", "Oslo"], "the quicker answered first");
    /// ```
    pub fn task(&self) -> Task<'_, A, M> {
        Task {
            cx: self,
            lane: None,
            key: None,
            how: PhantomData,
        }
    }
}

impl<'cx, A: 'static, M, P, R> Task<'cx, A, M, P, R> {
    fn with<Q, S>(self) -> Task<'cx, A, M, Q, S> {
        Task {
            cx: self.cx,
            lane: self.lane,
            key: self.key,
            how: PhantomData,
        }
    }

    /// Splits the lane by `key`: work under one key never gives way to work
    /// under another.
    pub fn keyed<K: Any + Eq>(mut self, key: K) -> Self {
        self.key = Some(Box::new(key));
        self
    }

    /// Names the lane by `K` instead of by what the work returns: for
    /// detached work, which returns nothing to tell lanes apart by, and for
    /// two lanes of one answer. A lane named by its answer is `.lane::<Answer>()`.
    pub fn lane<K: 'static>(mut self) -> Self {
        self.lane = Some(LaneId::of::<K>());
        self
    }

    /// Ends the lane named by [`lane`](Task::lane) - under its key when
    /// [`keyed`](Task::keyed), under every key when not: what runs in it is
    /// cut off, what waits is dropped, and neither answers.
    ///
    /// # Panics
    ///
    /// When no lane was named.
    #[track_caller]
    pub fn cancel(self) {
        let Some(lane) = self.lane else {
            panic!(
                "{} cancels a lane without naming it: say which with `.lane::<K>()` - \
                 a lane named by its answer is `.lane::<Answer>()`",
                short_type_name::<A>()
            );
        };

        let addr = &self.cx.addr;
        let key = self.key.as_deref();
        let ended = addr.lanes().borrow_mut().take(|open| {
            open.lane == lane && key.is_none_or(|key| open.is_keyed(key))
        });

        end(ended);
    }
}

impl<'cx, A: 'static, M, R> Task<'cx, A, M, Parallel, R> {
    /// New work replaces the work in flight: what runs is cut off, and its
    /// answer, if it was already on its way, is dropped.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use std::time::Duration;
    /// # use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # use guinea_core::ui_thread::UiLoop;
    /// # struct Weather(Rc<RefCell<Vec<&'static str>>>);
    /// # struct Fetch { city: &'static str, takes: Duration }
    /// # struct Fetched(&'static str);
    /// # guinea_macros::actor! { Weather { handlers { Fetch => { task Fetched } Fetched } } }
    /// # impl Handler<Fetched> for Weather {
    /// #     fn handle(&mut self, Fetched(city): Fetched, _cx: Cx<Self, Fetched>) {
    /// #         self.0.borrow_mut().push(city);
    /// #     }
    /// # }
    /// impl Handler<Fetch> for Weather {
    ///     fn handle(&mut self, Fetch { city, takes }: Fetch, cx: Cx<Self, Fetch>) {
    ///         cx.task().latest().spawn(async move {
    ///             tokio::time::sleep(takes).await;
    ///             Fetched(city)
    ///         });
    ///     }
    /// }
    /// # let ui = UiLoop::install();
    /// # let runtime = tokio::runtime::Runtime::new().unwrap();
    /// # let _entered = runtime.enter();
    /// # let tree = ScopeTree::new();
    /// # let shown = Rc::new(RefCell::new(Vec::new()));
    /// # let weather = Addr::new(Weather(shown.clone()), &Home::new(tree.scope()));
    /// # let fetch = |city: &'static str, ms: u64| weather.send(Fetch { city, takes: Duration::from_millis(ms) });
    ///
    /// fetch("Oslo", 200);
    /// fetch("Rome", 0);
    /// # assert!(ui.run_until(|| !shown.borrow().is_empty(), Duration::from_secs(5)));
    /// # assert!(!ui.run_until(|| shown.borrow().len() > 1, Duration::from_millis(400)));
    ///
    /// assert_eq!(*shown.borrow(), ["Rome"], "Oslo was cut off when Rome was asked for");
    /// ```
    pub fn latest(self) -> Task<'cx, A, M, Latest, R> {
        self.with()
    }

    /// The work in flight finishes; then only the newest that came meanwhile
    /// runs, and the ones between never start.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use std::time::Duration;
    /// # use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # use guinea_core::ui_thread::UiLoop;
    /// # struct Weather(Rc<RefCell<Vec<&'static str>>>);
    /// # struct Fetch { city: &'static str, takes: Duration }
    /// # struct Fetched(&'static str);
    /// # guinea_macros::actor! { Weather { handlers { Fetch => { task Fetched } Fetched } } }
    /// # impl Handler<Fetched> for Weather {
    /// #     fn handle(&mut self, Fetched(city): Fetched, _cx: Cx<Self, Fetched>) {
    /// #         self.0.borrow_mut().push(city);
    /// #     }
    /// # }
    /// impl Handler<Fetch> for Weather {
    ///     fn handle(&mut self, Fetch { city, takes }: Fetch, cx: Cx<Self, Fetch>) {
    ///         cx.task().conflate().spawn(async move {
    ///             tokio::time::sleep(takes).await;
    ///             Fetched(city)
    ///         });
    ///     }
    /// }
    /// # let ui = UiLoop::install();
    /// # let runtime = tokio::runtime::Runtime::new().unwrap();
    /// # let _entered = runtime.enter();
    /// # let tree = ScopeTree::new();
    /// # let shown = Rc::new(RefCell::new(Vec::new()));
    /// # let weather = Addr::new(Weather(shown.clone()), &Home::new(tree.scope()));
    /// # let fetch = |city: &'static str, ms: u64| weather.send(Fetch { city, takes: Duration::from_millis(ms) });
    ///
    /// fetch("Oslo", 200);
    /// fetch("Rome", 0);
    /// fetch("Paris", 0);
    /// # assert!(ui.run_until(|| shown.borrow().len() == 2, Duration::from_secs(5)));
    /// # assert!(!ui.run_until(|| shown.borrow().len() > 2, Duration::from_millis(400)));
    ///
    /// assert_eq!(*shown.borrow(), ["Oslo", "Paris"], "Rome gave way to Paris before it started");
    /// ```
    pub fn conflate(self) -> Task<'cx, A, M, Conflate, R> {
        self.with()
    }

    /// Nothing new while a piece is in flight: what comes meanwhile is
    /// dropped without being started.
    ///
    /// ```
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use std::time::Duration;
    /// # use guinea_core::actor::{Addr, Cx, Handler, Home};
    /// # use guinea_core::scope::ScopeTree;
    /// # use guinea_core::ui_thread::UiLoop;
    /// # struct Weather(Rc<RefCell<Vec<&'static str>>>);
    /// # struct Fetch { city: &'static str, takes: Duration }
    /// # struct Fetched(&'static str);
    /// # guinea_macros::actor! { Weather { handlers { Fetch => { task Fetched } Fetched } } }
    /// # impl Handler<Fetched> for Weather {
    /// #     fn handle(&mut self, Fetched(city): Fetched, _cx: Cx<Self, Fetched>) {
    /// #         self.0.borrow_mut().push(city);
    /// #     }
    /// # }
    /// impl Handler<Fetch> for Weather {
    ///     fn handle(&mut self, Fetch { city, takes }: Fetch, cx: Cx<Self, Fetch>) {
    ///         cx.task().exhaust().spawn(async move {
    ///             tokio::time::sleep(takes).await;
    ///             Fetched(city)
    ///         });
    ///     }
    /// }
    /// # let ui = UiLoop::install();
    /// # let runtime = tokio::runtime::Runtime::new().unwrap();
    /// # let _entered = runtime.enter();
    /// # let tree = ScopeTree::new();
    /// # let shown = Rc::new(RefCell::new(Vec::new()));
    /// # let weather = Addr::new(Weather(shown.clone()), &Home::new(tree.scope()));
    /// # let fetch = |city: &'static str, ms: u64| weather.send(Fetch { city, takes: Duration::from_millis(ms) });
    ///
    /// fetch("Oslo", 200);
    /// fetch("Rome", 0);
    /// # assert!(ui.run_until(|| !shown.borrow().is_empty(), Duration::from_secs(5)));
    /// # assert!(!ui.run_until(|| shown.borrow().len() > 1, Duration::from_millis(400)));
    /// assert_eq!(*shown.borrow(), ["Oslo"], "Rome came while Oslo was in flight");
    ///
    /// fetch("Paris", 0);
    /// # assert!(ui.run_until(|| shown.borrow().len() == 2, Duration::from_secs(5)));
    /// assert_eq!(*shown.borrow(), ["Oslo", "Paris"], "and Paris after it was done");
    /// ```
    pub fn exhaust(self) -> Task<'cx, A, M, Exhaust, R> {
        self.with()
    }
}

impl<'cx, A: 'static, M, P> Task<'cx, A, M, P, Reply> {
    /// Work that answers no one. Under a policy it needs a
    /// [`lane`](Task::lane): what it returns cannot name one.
    pub fn detached(self) -> Task<'cx, A, M, P, Detached> {
        self.with()
    }
}

impl<'cx, A: 'static, M, P: Policy> Task<'cx, A, M, P, Reply> {
    /// Starts `fut`. Cut off where it last awaited when it has to stop.
    ///
    /// # Panics
    ///
    /// When the lane was given another policy before.
    #[track_caller]
    pub fn spawn<Out, Fut>(self, fut: Fut)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
        Fut: Future<Output = Out> + Send + 'static,
    {
        self.reply(Location::caller(), move |_| fut, Ends::Dropped);
    }

    /// Starts what `work` makes of the token it is handed, for work that
    /// would rather hear it has to stop than be cut off: it is left to wind
    /// itself down, and what it returns then is for nobody.
    ///
    /// Under a policy that may hold work back, `work` is called when the
    /// work's turn comes, not now.
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
    /// # guinea_macros::actor! { Poller { handlers { Start => { task Tick } Tick } } }
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
    /// cx.task().spawn_with(|gone| async move {
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
    ///
    /// # Panics
    ///
    /// When the lane was given another policy before.
    #[track_caller]
    pub fn spawn_with<Out, Fut, F>(self, work: F)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        A::Flow: Allows<M, Out>,
        Fut: Future<Output = Out> + Send + 'static,
        F: FnOnce(Cancel) -> Fut + 'static,
    {
        self.reply(Location::caller(), work, Ends::Itself);
    }

    fn reply<Out, Fut, F>(self, at: &'static Location<'static>, work: F, ends: Ends)
    where
        Out: Send + 'static,
        A: Handler<Out> + ManagedActor,
        Fut: Future<Output = Out> + Send + 'static,
        F: FnOnce(Cancel) -> Fut + 'static,
    {
        let addr = &self.cx.addr;
        let reach = addr.reach();
        let lane = self.lane.unwrap_or(LaneId::of::<Out>());

        let spawning = Spawning {
            lane: Some(lane),
            key: self.key,
            kind: P::KIND,
            at,
        };
        launch(addr, spawning, short_type_name::<Out>(), move |traced| -> Start {
            Box::new(move |ticket: Ticket| {
                let fut = work(ticket.cancel.clone());
                crate::executor::spawn(run_reply(reach, traced, ticket, fut, ends));
            })
        });
    }
}

impl<'cx, A: 'static, M, P: Policy> Task<'cx, A, M, P, Detached> {
    /// [`Task::spawn`], answering no one.
    ///
    /// # Panics
    ///
    /// Under a policy, when no [`lane`](Task::lane) was named; and when the
    /// lane was given another policy before.
    #[track_caller]
    pub fn spawn<Fut>(self, fut: Fut)
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.detached_at(Location::caller(), move |_| fut, Ends::Dropped);
    }

    /// [`Task::spawn_with`], answering no one.
    ///
    /// # Panics
    ///
    /// As [`Task::spawn`].
    #[track_caller]
    pub fn spawn_with<Fut, F>(self, work: F)
    where
        Fut: Future<Output = ()> + Send + 'static,
        F: FnOnce(Cancel) -> Fut + 'static,
    {
        self.detached_at(Location::caller(), work, Ends::Itself);
    }

    fn detached_at<Fut, F>(self, at: &'static Location<'static>, work: F, ends: Ends)
    where
        Fut: Future<Output = ()> + Send + 'static,
        F: FnOnce(Cancel) -> Fut + 'static,
    {
        if P::KIND != Kind::Parallel && self.lane.is_none() {
            panic!(
                "{} spawns detached work under {:?} at {at} with no lane to hold it in: \
                 name one with `.lane::<K>()`",
                short_type_name::<A>(),
                P::KIND
            );
        }

        let addr = &self.cx.addr;
        let reach = addr.reach();

        let spawning = Spawning {
            lane: self.lane,
            key: self.key,
            kind: P::KIND,
            at,
        };
        launch(addr, spawning, "()", move |traced| -> Start {
            Box::new(move |ticket: Ticket| {
                let fut = work(ticket.cancel.clone());
                crate::executor::spawn(run_detached(reach, traced, ticket, fut, ends));
            })
        });
    }
}

/// One piece of work as the trace sees it: whose it is, what it owes them,
/// and when it was asked for.
///
/// The actor's id is what ties the work to a segment: it belongs where its
/// actor does, and the snapshot already says where that is.
#[derive(Clone, Copy)]
pub(crate) struct Traced {
    actor: &'static str,
    actor_id: u64,
    output: &'static str,
    asked: Instant,
    spawned: Cause,
}

impl Traced {
    fn spawn<A: 'static>(actor_id: usize, output: &'static str) -> Self {
        let actor = short_type_name::<A>();
        let actor_id = actor_id as u64;
        let spawned = trace::mark(|| Point::Spawn {
            actor,
            actor_id,
            output,
        });

        Self {
            actor,
            actor_id,
            output,
            asked: Instant::now(),
            spawned,
        }
    }

    fn took_us(&self) -> u64 {
        self.asked.elapsed().as_micros() as u64
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

    fn superseded(&self) -> Point {
        Point::Superseded {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
            took_us: self.took_us(),
        }
    }

    fn skipped(&self) -> Point {
        Point::Skipped {
            actor: self.actor,
            actor_id: self.actor_id,
            output: self.output,
        }
    }

    /// Records how the work ended, under its spawn, here on the UI thread.
    fn end(&self, point: Point) {
        trace::mark_under(Some(self.spawned), || point);
    }

    /// Records how the work ended, under its spawn, from whichever thread
    /// polled it last: [`crate::observability::mark_anywhere`] carries it to
    /// the thread being watched.
    fn end_anywhere(self, point: Point) {
        let _resumed = trace::resume(Some(self.spawned));

        crate::observability::mark_anywhere(move || point);
    }
}

/// What a run is handed when it starts: which run it is, and the token that
/// ends it.
#[derive(Clone)]
struct Ticket {
    run: Option<u64>,
    cancel: Cancel,
}

impl Ticket {
    /// For work in no lane: the actor's own token.
    fn free(cancel: Cancel) -> Self {
        Self { run: None, cancel }
    }

    /// How a run that does not answer ended, as its token says.
    fn cut(&self, traced: &Traced) -> Point {
        match self.cancel.reason() {
            Some(Reason::Superseded) => traced.superseded(),
            Some(Reason::Cancelled) | None => traced.cancelled(),
        }
    }
}

/// How work stops when its token is cancelled.
#[derive(Clone, Copy)]
enum Ends {
    /// It was handed the token and stops itself: it runs to its end.
    Itself,
    /// It never saw the token: cancelling drops it where it awaits.
    Dropped,
}

impl Ends {
    /// The work's output; `None` when it was dropped on cancellation.
    async fn wait<F: Future>(self, work: F, cancel: &Cancel) -> Option<F::Output> {
        match self {
            Ends::Itself => Some(work.await),
            Ends::Dropped => cancel.guard(work).await,
        }
    }
}

type Start = Box<dyn FnOnce(Ticket)>;

struct Spawning {
    lane: Option<LaneId>,
    key: Option<Box<dyn LaneKey>>,
    kind: Kind,
    at: &'static Location<'static>,
}

/// Records the spawn, and starts the work now, later or never, as its lane
/// says; never, once the actor is gone. On the UI thread.
fn launch<A: 'static>(
    addr: &Addr<A>,
    spawning: Spawning,
    output: &'static str,
    starting: impl FnOnce(Traced) -> Start,
) {
    let Spawning {
        lane,
        key,
        kind,
        at,
    } = spawning;

    if let Some(lane) = lane {
        addr.lanes()
            .borrow_mut()
            .declare(short_type_name::<A>(), lane, kind, at);
    }

    let traced = Traced::spawn::<A>(addr.id(), output);

    if addr.cancellation().is_cancelled() {
        traced.end(traced.cancelled());
        return;
    }

    let start = starting(traced);

    if kind == Kind::Parallel {
        start(Ticket::free(addr.cancellation()));
        return;
    }

    let lane = lane.expect("work under a policy has a lane: its answer's, or one named");

    let waiting = Waiting { traced, start };
    let admitted = addr
        .lanes()
        .borrow_mut()
        .admit(lane, key, kind, waiting, &addr.cancellation());

    if let Some(skipped) = admitted.skipped {
        skipped.traced.end(skipped.traced.skipped());
    }

    if let Some((waiting, ticket)) = admitted.starts {
        (waiting.start)(ticket);
    }
}

async fn run_reply<A, Out, Fut>(reach: Reach<A>, traced: Traced, ticket: Ticket, fut: Fut, ends: Ends)
where
    A: Handler<Out>,
    Out: Send + 'static,
    Fut: Future<Output = Out>,
{
    let unwinding = Unwinding::arm(reach, traced, &ticket);
    let running = trace::within(Some(traced.spawned), fut);
    let result = ends.wait(running, &ticket.cancel).await;
    drop(unwinding);

    let result = match result {
        Some(result) if !ticket.cancel.is_cancelled() => result,
        _ => {
            traced.end_anywhere(ticket.cut(&traced));
            return;
        }
    };

    invoke_on_ui(move || {
        let Some(addr) = reach.addr() else {
            traced.end(traced.cancelled());
            return;
        };

        if !addr.finish(&ticket) {
            traced.end(ticket.cut(&traced));
            return;
        }

        let settled = trace::mark_under(Some(traced.spawned), || traced.settled());
        addr.send_under(result, Some(settled));
    });
}

async fn run_detached<A, Fut>(reach: Reach<A>, traced: Traced, ticket: Ticket, fut: Fut, ends: Ends)
where
    A: 'static,
    Fut: Future<Output = ()>,
{
    let unwinding = Unwinding::arm(reach, traced, &ticket);
    let running = trace::within(Some(traced.spawned), fut);
    let ran = ends.wait(running, &ticket.cancel).await.is_some();
    drop(unwinding);

    if !ran || ticket.cancel.is_cancelled() {
        traced.end_anywhere(ticket.cut(&traced));
        return;
    }

    if ticket.run.is_none() {
        traced.end_anywhere(traced.settled());
        return;
    }

    invoke_on_ui(move || {
        let ended = match reach.addr().is_some_and(|addr| addr.finish(&ticket)) {
            true => traced.settled(),
            false => ticket.cut(&traced),
        };
        traced.end(ended);
    });
}

/// Held while a run's work is polled: when the work panics, it frees the
/// run's lane on the UI thread and ends it in the trace as cancelled, while
/// the panic goes on to whatever polled it. Posting is tried, not insisted
/// on: a second panic during the unwind would abort.
struct Unwinding<A: 'static> {
    reach: Reach<A>,
    traced: Traced,
    ticket: Ticket,
}

impl<A: 'static> Unwinding<A> {
    fn arm(reach: Reach<A>, traced: Traced, ticket: &Ticket) -> Self {
        Self {
            reach,
            traced,
            ticket: ticket.clone(),
        }
    }
}

impl<A: 'static> Drop for Unwinding<A> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }

        tracing::error!(
            "{}'s work for {} panicked: it ends as cancelled, and its lane takes work again",
            self.traced.actor,
            self.traced.output
        );

        let (reach, traced, ticket) = (self.reach, self.traced, self.ticket.clone());
        let posted = try_invoke_on_ui(move || {
            traced.end(traced.cancelled());

            if let Some(addr) = reach.addr() {
                addr.finish(&ticket);
            }
        });

        if posted.is_err() {
            traced.end_anywhere(traced.cancelled());
        }
    }
}

impl<A: 'static> Addr<A> {
    /// Lets the lane of the run `ticket` was handed take its next piece; on
    /// the UI thread, before the run's answer is handled. `false` when the run
    /// is no longer its lane's, and its answer is for nobody.
    fn finish(&self, ticket: &Ticket) -> bool {
        let Some(run) = ticket.run else {
            return true;
        };

        let finished = self.lanes().borrow_mut().finish(run, &self.cancellation());

        match finished {
            Finished::Stale => false,
            Finished::Freed => true,
            Finished::Next(waiting, ticket) => {
                (waiting.start)(ticket);
                true
            }
        }
    }

    /// Drops what waits in its lanes, as the actor goes: what runs ends with
    /// the actor's own token.
    pub(crate) fn close_lanes(&self) {
        let closed = self.lanes().borrow_mut().take(|_| true);
        end(closed);
    }
}

/// Where an actor keeps the lanes its policies made. Nothing until a policy
/// is used.
#[derive(Default)]
pub(crate) struct Lanes {
    declared: Vec<Declared>,
    open: Vec<Open>,
    runs: u64,
}

struct Declared {
    lane: LaneId,
    kind: Kind,
    at: &'static Location<'static>,
}

/// A lane with work in flight. One with none is not kept.
struct Open {
    lane: LaneId,
    key: Option<Box<dyn LaneKey>>,
    running: Run,
    /// The newest piece that came while `running` ran, under `conflate`.
    waiting: Option<Waiting>,
}

impl Open {
    fn is(&self, lane: LaneId, key: Option<&dyn LaneKey>) -> bool {
        self.lane == lane
            && match (&self.key, key) {
                (None, None) => true,
                (Some(mine), Some(theirs)) => mine.same(theirs),
                _ => false,
            }
    }

    fn is_keyed(&self, key: &dyn LaneKey) -> bool {
        self.key.as_ref().is_some_and(|mine| mine.same(key))
    }
}

struct Run {
    run: u64,
    cancel: Cancel,
}

struct Waiting {
    traced: Traced,
    start: Start,
}

struct Admitted {
    starts: Option<(Waiting, Ticket)>,
    skipped: Option<Waiting>,
}

/// What a lane makes of one of its runs ending.
enum Finished {
    /// The run is not its lane's any more: a newer one replaced it.
    Stale,
    /// Nothing waited: the lane is free.
    Freed,
    /// What waited, ready to start as the lane's next run.
    Next(Waiting, Ticket),
}

impl Lanes {
    fn declare(&mut self, actor: &'static str, lane: LaneId, kind: Kind, at: &'static Location<'static>) {
        let Some(before) = self.declared.iter().find(|declared| declared.lane == lane) else {
            self.declared.push(Declared { lane, kind, at });
            return;
        };

        if before.kind != kind {
            panic!(
                "{actor}'s lane {} runs {:?} work since {}, and {kind:?} work spawned into it at {at}: \
                 one lane keeps one policy - give one of them its own `.lane::<K>()`",
                lane.name, before.kind, before.at
            );
        }
    }

    fn run(&mut self, parent: &Cancel) -> (Run, Ticket) {
        self.runs += 1;
        let cancel = parent.child();

        let run = Run {
            run: self.runs,
            cancel: cancel.clone(),
        };
        let ticket = Ticket {
            run: Some(self.runs),
            cancel,
        };
        (run, ticket)
    }

    fn admit(
        &mut self,
        lane: LaneId,
        key: Option<Box<dyn LaneKey>>,
        kind: Kind,
        waiting: Waiting,
        parent: &Cancel,
    ) -> Admitted {
        let found = self.open.iter().position(|open| open.is(lane, key.as_deref()));
        let Some(at) = found else {
            let (running, ticket) = self.run(parent);
            self.open.push(Open {
                lane,
                key,
                running,
                waiting: None,
            });

            return Admitted {
                starts: Some((waiting, ticket)),
                skipped: None,
            };
        };

        match kind {
            Kind::Latest => {
                let (running, ticket) = self.run(parent);
                let replaced = std::mem::replace(&mut self.open[at].running, running);
                replaced.cancel.cancel_because(Reason::Superseded);

                Admitted {
                    starts: Some((waiting, ticket)),
                    skipped: None,
                }
            }
            Kind::Conflate => Admitted {
                starts: None,
                skipped: self.open[at].waiting.replace(waiting),
            },
            Kind::Exhaust => Admitted {
                starts: None,
                skipped: Some(waiting),
            },
            Kind::Parallel => unreachable!("work in no lane starts where it is spawned"),
        }
    }

    /// Ends the run `run`: the lane goes free, or hands itself to what waited.
    fn finish(&mut self, run: u64, parent: &Cancel) -> Finished {
        let Some(at) = self.open.iter().position(|open| open.running.run == run) else {
            return Finished::Stale;
        };

        let Some(next) = self.open[at].waiting.take() else {
            self.open.swap_remove(at);
            return Finished::Freed;
        };

        let (running, ticket) = self.run(parent);
        self.open[at].running = running;
        Finished::Next(next, ticket)
    }

    fn take(&mut self, which: impl Fn(&Open) -> bool) -> Vec<Open> {
        let (taken, kept) = std::mem::take(&mut self.open).into_iter().partition(which);
        self.open = kept;
        taken
    }
}

/// Ends lanes taken out of an actor: what runs is cut off, what waits is
/// dropped unstarted. Outside the borrow, since dropping work runs its code.
fn end(lanes: Vec<Open>) {
    for open in lanes {
        open.running.cancel.cancel();

        if let Some(waiting) = open.waiting {
            waiting.traced.end(waiting.traced.cancelled());
        }
    }
}

#[cfg(all(test, feature = "test-utils"))]
mod tests;
