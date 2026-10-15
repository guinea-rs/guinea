//! Sources, owned by the scope that opened them.
//!
//! A source is a stream whose items its scope hands, one by one and on the
//! UI thread, to whoever it names: an actor, or a function. Each item arrives
//! as a root of its own. A timer is one - a source of [`Ticks`].
//!
//! The scope holds a source beside its actors and what it hears: it pours
//! for as long as the scope lives, ends the moment the scope goes, and is
//! listed for devtools with where it was opened. One that runs dry lets
//! itself go.
//!
//! Streams are pulled through the same executor as background work, so a
//! source needs the runtime `cx.task()` needs - and under a test harness
//! ticks wait on the test's clock.

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::panic::Location;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use futures_core::Stream;
use parking_lot::Mutex;

use crate::actor::{Cancel, Receives};
use crate::observability::observer::{self, Event};
use crate::scope::{Awake, Scope, ScopeId, Section, next_key};
use crate::short_type_name;
use crate::trace::{self, Cause, Point};
use crate::ui_thread::invoke_on_ui;

/// How long between ticks: a [`Duration`], or what a changing value says.
pub struct Period(Pace);

enum Pace {
    Fixed(Duration),
    Following(Arc<dyn Follows>),
}

trait Follows: Send + Sync {
    fn now(&self) -> Option<Duration>;
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send>;
}

struct Followed<C, F> {
    source: C,
    period: F,
}

impl<C, F> Follows for Followed<C, F>
where
    C: Changing,
    F: Fn(C::Value) -> Duration + Send + Sync + 'static,
{
    fn now(&self) -> Option<Duration> {
        self.source.now().map(&self.period)
    }

    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        self.source.watch(changed)
    }
}

/// A value that changes and says so: what a period [follows](Period::follows).
///
/// A store's reactive value gets it from the crate that knows both - for
/// amethystate's `Field` and `ReactiveCell`, `.changing()` from
/// `amethystate-guinea` - and anything else that can be read and watched
/// implements it by hand.
pub trait Changing: Send + Sync + 'static {
    type Value;

    /// What it is now, or `None` while there is nothing to read.
    fn now(&self) -> Option<Self::Value>;

    /// Calls `changed` every time it changes, from whichever thread the
    /// change happens on, until what this returns is dropped.
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send>;
}

impl Period {
    /// As long as `source` says, through `period`: a change cuts the wait
    /// under way short, and the next tick comes a new period after it. While
    /// `source` has nothing to read, there is no tick.
    ///
    /// ```
    /// # use std::any::Any;
    /// # use std::cell::Cell;
    /// # use std::rc::Rc;
    /// # use std::sync::{Arc, Mutex};
    /// # use std::time::Duration;
    /// # use guinea_core::scope::ScopeTree;
    /// # use guinea_core::sources::{Changing, Period};
    /// # use guinea_core::ui_thread::UiLoop;
    /// #[derive(Clone, Default)]
    /// struct Interval(Arc<Mutex<(Option<u64>, Vec<Box<dyn Fn() + Send + Sync>>)>>);
    ///
    /// impl Interval {
    ///     fn set(&self, ms: u64) {
    ///         let mut interval = self.0.lock().unwrap();
    ///         interval.0 = Some(ms);
    ///         interval.1.iter().for_each(|changed| changed());
    ///     }
    /// }
    ///
    /// impl Changing for Interval {
    ///     type Value = u64;
    ///
    ///     fn now(&self) -> Option<u64> {
    ///         self.0.lock().unwrap().0
    ///     }
    ///
    ///     fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
    ///         self.0.lock().unwrap().1.push(changed);
    ///         Box::new(())
    ///     }
    /// }
    ///
    /// # let ui = UiLoop::install();
    /// # let runtime = tokio::runtime::Runtime::new().unwrap();
    /// # let _entered = runtime.enter();
    /// # let tree = ScopeTree::new();
    /// let interval = Interval::default();
    /// let ticks = Rc::new(Cell::new(0));
    /// let counted = ticks.clone();
    /// tree.scope().section().repeat(
    ///     Period::follows(interval.clone(), Duration::from_millis),
    ///     move || counted.set(counted.get() + 1),
    /// );
    ///
    /// assert!(!ui.run_until(|| ticks.get() > 0, Duration::from_millis(50)), "nothing to read yet");
    ///
    /// interval.set(5);
    /// assert!(ui.run_until(|| ticks.get() >= 3, Duration::from_secs(5)));
    /// ```
    pub fn follows<C: Changing>(
        source: C,
        period: impl Fn(C::Value) -> Duration + Send + Sync + 'static,
    ) -> Self {
        Period(Pace::Following(Arc::new(Followed { source, period })))
    }

    /// The wait before the next tick, or `None` while there is nothing to
    /// wait by.
    fn next(&self) -> Option<Duration> {
        match &self.0 {
            Pace::Fixed(period) => Some(*period),
            Pace::Following(follows) => follows.now(),
        }
    }
}

impl From<Duration> for Period {
    fn from(period: Duration) -> Self {
        Period(Pace::Fixed(period))
    }
}

/// Ticks, one every [`Period`] or one only: what a timer pours.
///
/// The next wait starts when the tick before it is taken, so a scope that
/// sleeps through several gets one when it wakes.
pub struct Ticks {
    period: Period,
    once: bool,
    done: bool,
    wait: Option<Pin<Box<tokio::time::Sleep>>>,
    changed: Option<Arc<Changed>>,
    _watching: Option<Box<dyn Any + Send>>,
}

#[derive(Default)]
struct Changed {
    changed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Changed {
    fn tell(&self) {
        self.changed.store(true, Ordering::SeqCst);

        if let Some(waker) = self.waker.lock().take() {
            waker.wake();
        }
    }
}

impl Ticks {
    /// A tick every `period`, for ever.
    pub fn every(period: impl Into<Period>) -> Self {
        let period = period.into();

        let (changed, watching) = match &period.0 {
            Pace::Fixed(_) => (None, None),
            Pace::Following(follows) => {
                let changed = Arc::new(Changed::default());
                let told = changed.clone();
                let watching = follows.watch(Box::new(move || told.tell()));
                (Some(changed), Some(watching))
            }
        };

        Self {
            period,
            once: false,
            done: false,
            wait: None,
            changed,
            _watching: watching,
        }
    }

    /// One tick, `delay` from when it is first asked for.
    pub fn once(delay: Duration) -> Self {
        Self {
            period: Period(Pace::Fixed(delay)),
            once: true,
            done: false,
            wait: None,
            changed: None,
            _watching: None,
        }
    }
}

impl Stream for Ticks {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
        let this = &mut *self;
        if this.done {
            return Poll::Ready(None);
        }

        if let Some(changed) = &this.changed {
            *changed.waker.lock() = Some(cx.waker().clone());

            if changed.changed.swap(false, Ordering::SeqCst) {
                this.wait = None;
            }
        }

        if this.wait.is_none() {
            let Some(after) = this.period.next() else {
                return Poll::Pending;
            };
            this.wait = Some(Box::pin(tokio::time::sleep(after)));
        }

        let wait = this.wait.as_mut().expect("armed above");
        if wait.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }

        this.wait = None;
        this.done = this.once;
        Poll::Ready(Some(()))
    }
}

/// What is known about an open source.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub id: u64,
    pub name: Option<&'static str>,
    /// Where it was opened.
    pub place: &'static Location<'static>,
    /// The feature that opened it.
    pub feature: Option<&'static str>,
    /// The scope that holds it.
    pub scope: ScopeId,
    /// Who its items go to: an actor's name, or `fn`.
    pub to: &'static str,
    /// What its items come as.
    pub output: &'static str,
}

struct Entry {
    info: SourceInfo,
    scope: Scope,
    cancel: Cancel,
    /// An `Rc<dyn Fn(T)>`, for the `T` the source pours.
    receive: Box<dyn Any>,
    /// A `VecDeque<T>` of what reached the UI thread while the scope slept,
    /// handed over at the wake.
    held: Option<Box<dyn Any>>,
    /// How the source ended, while what it held waits for the wake.
    ended: Option<Closing>,
}

type Closed = Option<Box<dyn FnOnce() + Send>>;

struct Closing {
    gone: bool,
    closed: Closed,
}

/// A source as its scope holds it: dropping it ends the source.
pub(crate) struct Pouring(Rc<RefCell<Entry>>);

impl Drop for Pouring {
    fn drop(&mut self) {
        let (id, scope) = {
            let entry = self.0.borrow();
            entry.cancel.cancel();
            (entry.info.id, entry.scope)
        };

        observer::tell(scope, || Event::SourceClosed { id });
    }
}

impl Pouring {
    fn entry(&self) -> Rc<RefCell<Entry>> {
        self.0.clone()
    }

    fn info(&self) -> SourceInfo {
        self.0.borrow().info.clone()
    }
}

/// A source that was opened, for saying more about it.
///
/// Holding it does not keep the source open, and dropping it does not end
/// it: the scope that holds it does both.
#[derive(Clone)]
pub struct Source {
    entry: Weak<RefCell<Entry>>,
}

impl Source {
    /// What devtools call it, instead of where it was opened.
    pub fn named(self, name: &'static str) -> Self {
        if let Some(entry) = self.entry.upgrade() {
            let (id, scope) = {
                let mut entry = entry.borrow_mut();
                entry.info.name = Some(name);
                (entry.info.id, entry.scope)
            };
            observer::tell(scope, || Event::SourceNamed { id, name });
        }
        self
    }

    /// Its id while it is open.
    pub fn id(&self) -> Option<u64> {
        self.entry.upgrade().map(|entry| entry.borrow().info.id)
    }
}

/// How a source is opened, besides its stream.
pub(crate) struct Opening<T> {
    pub(crate) place: &'static Location<'static>,
    pub(crate) to: &'static str,
    /// What its items come to `to` as.
    pub(crate) output: &'static str,
    /// Ends the source when cancelled, beside its scope going.
    pub(crate) cancel: Cancel,
    pub(crate) receive: Rc<dyn Fn(T)>,
    /// Run on the UI thread when the source ends, whether its scope still
    /// holds it or not.
    pub(crate) closed: Closed,
}

/// Opens `source` in `section`'s scope, its items made into what the
/// receiver takes by `into` where they are pulled.
///
/// # Panics
///
/// When the scope is gone.
pub(crate) fn open<S, T>(
    section: Section,
    source: S,
    into: impl FnMut(S::Item) -> T + Send + 'static,
    opening: Opening<T>,
) -> Source
where
    S: Stream + Send + 'static,
    T: Send + 'static,
{
    let Opening {
        place,
        to,
        output,
        cancel,
        receive,
        closed,
    } = opening;

    let scope = section.scope();
    let key = next_key();

    let entry = Rc::new(RefCell::new(Entry {
        info: SourceInfo {
            id: key as u64,
            name: None,
            place,
            feature: section.feature(),
            scope: scope.id(),
            to,
            output,
        },
        scope,
        cancel: cancel.clone(),
        receive: Box::new(receive),
        held: None,
        ended: None,
    }));
    let handle = Source {
        entry: Rc::downgrade(&entry),
    };
    let told = entry.clone();
    scope.hold_source(key, Pouring(entry));
    observer::tell(scope, || Event::SourceOpened(told.borrow().info.clone()));

    let feed = Feed {
        source: key as u64,
        to,
        output,
        started: Instant::now(),
    };
    let opened = trace::mark(|| Point::Source {
        source: feed.source,
        to,
        output,
        file: place.file(),
        line: place.line(),
    });

    let awake = scope.awake();
    crate::executor::spawn(async move {
        let pouring = trace::within(None, pour(scope, key, source, into, awake, feed.clone()));
        let ran_dry = cancel.guard(pouring).await.is_some();
        let gone = !ran_dry || cancel.is_cancelled();

        feed.closed(opened, gone);
        invoke_on_ui(move || close::<T>(scope, key, Closing { gone, closed }));
    });

    handle
}

/// One source as the trace sees it, from whichever thread pulls it.
#[derive(Clone)]
struct Feed {
    source: u64,
    to: &'static str,
    output: &'static str,
    started: Instant,
}

impl Feed {
    /// Opens a pull of the source: what the stream does until it has its next
    /// item, runs dry or is dropped is recorded under it.
    fn pull(&self) -> Pulling {
        let id = trace::reserve();
        let (source, to, output) = (self.source, self.to, self.output);
        crate::observability::begin_anywhere(id, None, move || Point::Pull { source, to, output });

        Pulling {
            id,
            started: Instant::now(),
        }
    }

    fn closed(&self, opened: Cause, gone: bool) {
        let point = Point::Closed {
            source: self.source,
            to: self.to,
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

/// Hands each item of `source` to the UI thread, until the source runs dry -
/// pulling only while the scope is awake. Gives way after each item, so a
/// source that is always ready still notices it has to stop.
async fn pour<S, T>(
    scope: Scope,
    key: usize,
    source: S,
    mut into: impl FnMut(S::Item) -> T,
    awake: Awake,
    feed: Feed,
) where
    S: Stream,
    T: Send + 'static,
{
    let mut source = std::pin::pin!(source);

    loop {
        awake.wait().await;

        let next = async {
            let pulling = feed.pull();

            std::future::poll_fn(|cx| {
                let _pulling = trace::resume(Some(pulling.id));
                source.as_mut().poll_next(cx)
            })
            .await
        };
        let Some(item) = next.await else {
            break;
        };
        let item = into(item);

        awake.wait().await;
        invoke_on_ui(move || arrive(scope, key, item));

        crate::executor::yield_now().await;
    }
}

/// An item reached the UI thread: handed over, unless its source is gone -
/// or held for the wake, when the scope fell asleep on the way, behind
/// whatever is held already.
fn arrive<T: Send + 'static>(scope: Scope, key: usize, item: T) {
    let Some(entry) = scope.held_source(key, Pouring::entry) else {
        return;
    };

    if scope.is_awake() && entry.borrow().held.is_none() {
        deliver(&entry, item);
        return;
    }

    let mut held = entry.borrow_mut();
    match held.held.as_mut() {
        Some(waiting) => held_items::<T>(waiting).push_back(item),
        None => {
            held.held = Some(Box::new(VecDeque::from([item])));
            release_at_wake::<T>(scope, key, held.cancel.clone());
        }
    }
}

fn held_items<T: 'static>(held: &mut Box<dyn Any>) -> &mut VecDeque<T> {
    held.downcast_mut::<VecDeque<T>>()
        .expect("a source holds what it pours")
}

fn release_at_wake<T: Send + 'static>(scope: Scope, key: usize, cancel: Cancel) {
    let awake = scope.awake();

    crate::executor::spawn(async move {
        if cancel.guard(awake.wait()).await.is_some() {
            invoke_on_ui(move || release::<T>(scope, key));
        }
    });
}

/// Hands over what was held while the scope slept, in order, for as long as
/// it stays awake; then ends the source, if it ended meanwhile.
fn release<T: Send + 'static>(scope: Scope, key: usize) {
    let Some(entry) = scope.held_source(key, Pouring::entry) else {
        return;
    };

    loop {
        if !scope.is_awake() {
            let cancel = entry.borrow().cancel.clone();
            release_at_wake::<T>(scope, key, cancel);
            return;
        }

        let next = {
            let mut entry = entry.borrow_mut();
            let next = entry.held.as_mut().and_then(|held| held_items::<T>(held).pop_front());
            if next.is_none() {
                entry.held = None;
            }
            next
        };
        let Some(item) = next else {
            break;
        };

        deliver(&entry, item);
    }

    let ended = entry.borrow_mut().ended.take();
    if let Some(ended) = ended {
        close::<T>(scope, key, ended);
    }
}

fn deliver<T: 'static>(entry: &Rc<RefCell<Entry>>, item: T) {
    let (receive, arrived) = {
        let entry = entry.borrow();
        let receive = entry
            .receive
            .downcast_ref::<Rc<dyn Fn(T)>>()
            .expect("a source pours what its receiver takes")
            .clone();
        let info = &entry.info;
        let arrived = Point::Arrived {
            source: info.id,
            name: info.name,
            to: info.to,
            output: info.output,
        };
        (receive, arrived)
    };

    let _root = trace::resume(None);
    let _arrived = trace::enter(move || arrived);
    receive(item);
}

/// The source ended: its scope lets go of it - once what it held for the
/// wake is handed over, when it ran dry.
fn close<T: Send + 'static>(scope: Scope, key: usize, closing: Closing) {
    if !closing.gone
        && let Some(entry) = scope.held_source(key, Pouring::entry)
        && entry.borrow().held.is_some()
    {
        entry.borrow_mut().ended = Some(closing);
        return;
    }

    scope.let_go_of(key);

    if let Some(closed) = closing.closed {
        closed();
    }
}

impl Section {
    /// Hands `to` every item `source` yields, for as long as this section's
    /// scope lives: `to` is an actor that handles it, or a function that
    /// takes it.
    ///
    /// # Panics
    ///
    /// When the scope is gone.
    #[track_caller]
    pub fn pour<S>(&self, source: S, to: impl Receives<S::Item>) -> Source
    where
        S: Stream + Send + 'static,
        S::Item: Send + 'static,
    {
        let opening = Opening {
            place: Location::caller(),
            to: to.name(),
            output: short_type_name::<S::Item>(),
            cancel: Cancel::new(),
            receive: Rc::new(move |item| to.receive(item)),
            closed: None,
        };
        open(*self, source, |item| item, opening)
    }

    /// Hands `to` a `message()` every `period`, for as long as this
    /// section's scope lives: `to` is an actor that handles it, or a
    /// function that takes it.
    ///
    /// # Panics
    ///
    /// When the scope is gone.
    #[track_caller]
    pub fn every<M: 'static>(
        &self,
        period: impl Into<Period>,
        message: impl Fn() -> M + 'static,
        to: impl Receives<M>,
    ) -> Source {
        let opening = Opening {
            place: Location::caller(),
            to: to.name(),
            output: short_type_name::<M>(),
            cancel: Cancel::new(),
            receive: Rc::new(move |()| to.receive(message())),
            closed: None,
        };
        open(*self, Ticks::every(period), |tick| tick, opening)
    }

    /// Runs `run` every `period`, for as long as this section's scope lives.
    ///
    /// # Panics
    ///
    /// When the scope is gone.
    #[track_caller]
    pub fn repeat(&self, period: impl Into<Period>, run: impl FnMut() + 'static) -> Source {
        let run = RefCell::new(run);
        let opening = Opening {
            place: Location::caller(),
            to: "fn",
            output: "()",
            cancel: Cancel::new(),
            receive: Rc::new(move |()| (run.borrow_mut())()),
            closed: None,
        };
        open(*self, Ticks::every(period), |tick| tick, opening)
    }
}

impl Scope {
    /// The sources this scope and every scope under it hold, that devtools
    /// may see, oldest first.
    pub fn sources(&self) -> Vec<SourceInfo> {
        let mut sources = Vec::new();
        for scope in self.subtree() {
            scope.each_source(|pouring| sources.push(pouring.info()));
        }

        sources.sort_by_key(|source| source.id);
        sources
    }

    /// The sources this scope holds itself, oldest first - not those of the
    /// scopes under it.
    pub fn sources_here(&self) -> Vec<SourceInfo> {
        let mut sources = Vec::new();
        self.each_source(|pouring| sources.push(pouring.info()));
        sources
    }
}

#[cfg(all(test, feature = "test-utils"))]
mod tests;
