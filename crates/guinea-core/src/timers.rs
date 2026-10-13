//! Timers, owned by the scope that set them up.
//!
//! A timer is one of the things a scope holds, beside its actors and what it
//! hears: it lives as long as the scope, ticks only while the scope is awake,
//! and its wait ends the moment the scope goes. There is no timer without an
//! owner. Each remembers where it was set up, which is how devtools tell one
//! from another, and ticks on the UI thread.
//!
//! They wait on `tokio::time`, through the same executor as background work,
//! so they need the runtime `cx.task()` needs - and under a test harness they
//! wait on the test's clock and tick when it is advanced.

use std::any::Any;
use std::cell::RefCell;
use std::panic::Location;
use std::rc::{Rc, Weak};
use std::time::Duration;

use crate::actor::Cancel;
use crate::observability::changes::{self, Change};
use crate::scope::{Scope, Section, next_key};
use crate::ui_thread::invoke_on_ui;

/// How long between ticks.
pub enum Period {
    Fixed(Duration),
    /// See [`Period::follows`].
    Following {
        now: Box<dyn Fn() -> Option<Duration>>,
        watch: Box<dyn Fn(Box<dyn Fn() + Send + Sync>) -> Box<dyn Any>>,
    },
}

/// A value that changes and says so: what a period [follows](Period::follows).
///
/// A store's reactive value gets it from the crate that knows both - for
/// amethystate's `Field` and `ReactiveCell`, `.changing()` from
/// `amethystate-guinea` - and anything else that can be read and watched
/// implements it by hand.
pub trait Changing: 'static {
    type Value;

    /// What it is now, or `None` while there is nothing to read.
    fn now(&self) -> Option<Self::Value>;

    /// Calls `changed` every time it changes, from whichever thread the
    /// change happens on, until what this returns is dropped.
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any>;
}

impl Period {
    /// As long as `source` says, through `period`: a change cuts the wait
    /// under way short, and the next tick comes a new period after it. While
    /// `source` has nothing to read, the timer does not tick.
    ///
    /// ```
    /// # use std::any::Any;
    /// # use std::cell::Cell;
    /// # use std::rc::Rc;
    /// # use std::sync::{Arc, Mutex};
    /// # use std::time::Duration;
    /// # use guinea_core::scope::ScopeTree;
    /// # use guinea_core::timers::{Changing, Period};
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
    ///     fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any> {
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
    pub fn follows<C: Changing>(source: C, period: impl Fn(C::Value) -> Duration + 'static) -> Self {
        let source = Rc::new(source);
        let watched = source.clone();

        Period::Following {
            now: Box::new(move || source.now().map(&period)),
            watch: Box::new(move |changed| watched.watch(changed)),
        }
    }

    /// The wait before the next tick, or `None` while there is nothing to
    /// wait by.
    fn next(&self) -> Option<Duration> {
        match self {
            Period::Fixed(period) => Some(*period),
            Period::Following { now, .. } => now(),
        }
    }

    /// Starts watching what it follows, for the timer under `key` in `scope`:
    /// a change starts its wait again on the UI thread, wherever the change
    /// happened.
    fn watch(&self, scope: Scope, key: usize) -> Option<Box<dyn Any>> {
        match self {
            Period::Following { watch, .. } => {
                Some(watch(Box::new(move || invoke_on_ui(move || follow(scope, key)))))
            }
            Period::Fixed(_) => None,
        }
    }
}

impl From<Duration> for Period {
    fn from(period: Duration) -> Self {
        Period::Fixed(period)
    }
}

/// What is known about a running timer.
#[derive(Clone, Debug)]
pub struct TimerInfo {
    pub id: u64,
    pub name: Option<&'static str>,
    /// Where it was set up.
    pub place: &'static Location<'static>,
    /// The feature that set it up.
    pub feature: Option<&'static str>,
    /// The scope that owns it.
    pub scope: Scope,
    /// The period of the wait under way, or of the last one.
    pub period: Duration,
}

struct Entry {
    info: TimerInfo,
    period: Period,
    run: Box<dyn FnMut()>,
    /// Ends the wait under way: when the timer goes, and when a new wait
    /// takes its place.
    waiting: Cancel,
    /// What keeps the value a [`Period::Following`] follows watched.
    _watching: Option<Box<dyn Any>>,
    traced: bool,
}

/// A timer as its scope holds it: dropping it stops the timer.
pub(crate) struct Ticking(Rc<RefCell<Entry>>);

impl Drop for Ticking {
    fn drop(&mut self) {
        let (id, traced) = {
            let entry = self.0.borrow();
            entry.waiting.cancel();
            (entry.info.id, entry.traced)
        };

        if traced {
            changes::changed(|| Change::TimerStopped { id });
        }
    }
}

impl Ticking {
    fn entry(&self) -> Rc<RefCell<Entry>> {
        self.0.clone()
    }

    fn info(&self) -> Option<TimerInfo> {
        let entry = self.0.borrow();
        entry.traced.then(|| entry.info.clone())
    }
}

/// A timer that was set up, for saying more about it.
///
/// Holding it does not keep the timer running, and dropping it does not stop
/// it: the scope that set it up does both.
#[derive(Clone)]
pub struct Timer {
    entry: Weak<RefCell<Entry>>,
}

impl Timer {
    /// What devtools call it, instead of where it was set up.
    pub fn named(self, name: &'static str) -> Self {
        if let Some(entry) = self.entry.upgrade() {
            entry.borrow_mut().info.name = Some(name);
        }
        self
    }

    /// Leaves no trace and does not show up in devtools: for tooling that
    /// watches the application and must not be seen in what it watches.
    pub fn untraced(self) -> Self {
        let Some(entry) = self.entry.upgrade() else {
            return self;
        };

        let (id, was_traced) = {
            let mut entry = entry.borrow_mut();
            (entry.info.id, std::mem::replace(&mut entry.traced, false))
        };
        if was_traced {
            changes::changed(|| Change::TimerStopped { id });
        }

        self
    }

    pub fn id(&self) -> Option<u64> {
        self.entry.upgrade().map(|entry| entry.borrow().info.id)
    }
}

impl Section {
    /// Runs `run` every `period`, for as long as this section's scope lives.
    ///
    /// # Panics
    ///
    /// When the scope is gone.
    #[track_caller]
    pub fn repeat(&self, period: impl Into<Period>, run: impl FnMut() + 'static) -> Timer {
        let place = Location::caller();
        let scope = self.scope();

        let key = next_key();
        let period = period.into();
        let watching = period.watch(scope, key);

        let entry = Rc::new(RefCell::new(Entry {
            info: TimerInfo {
                id: key as u64,
                name: None,
                place,
                feature: self.feature(),
                scope,
                period: Duration::ZERO,
            },
            period,
            run: Box::new(run),
            waiting: Cancel::new(),
            _watching: watching,
            traced: true,
        }));

        let timer = Timer {
            entry: Rc::downgrade(&entry),
        };
        scope.hold_timer(key, Ticking(entry.clone()));
        changes::changed(|| Change::TimerStarted { id: key as u64 });
        wait(scope, key, &mut entry.borrow_mut());

        timer
    }
}

impl Scope {
    /// The timers this scope and every scope under it run, that devtools may
    /// see, oldest first.
    pub fn timers(&self) -> Vec<TimerInfo> {
        let mut timers = Vec::new();
        for scope in self.subtree() {
            scope.each_timer(|ticking| timers.extend(ticking.info()));
        }

        timers.sort_by_key(|timer| timer.id);
        timers
    }

    /// The timers this scope runs itself, oldest first - not those of the
    /// scopes under it.
    pub fn timers_here(&self) -> Vec<TimerInfo> {
        let mut timers = Vec::new();
        self.each_timer(|ticking| timers.extend(ticking.info()));
        timers
    }
}

/// Starts the next wait of the timer under `key` in `scope`, ending the one
/// under way. With nothing to wait by, there is no next wait until what the
/// period follows changes.
fn wait(scope: Scope, key: usize, entry: &mut Entry) {
    entry.waiting.cancel();
    let waiting = Cancel::new();
    entry.waiting = waiting.clone();

    let Some(after) = entry.period.next() else {
        return;
    };
    entry.info.period = after;

    crate::executor::spawn(async move {
        if waiting.guard(tokio::time::sleep(after)).await.is_none() {
            return;
        }
        invoke_on_ui(move || tick(scope, key, waiting));
    });
}

fn tick(scope: Scope, key: usize, waited: Cancel) {
    if waited.is_cancelled() {
        return;
    }
    let Some(entry) = scope.held_timer(key, Ticking::entry) else {
        return;
    };

    if scope.is_awake() {
        let (id, traced, name, place) = {
            let entry = entry.borrow();
            (entry.info.id, entry.traced, entry.info.name, entry.info.place)
        };
        let mut run = std::mem::replace(&mut entry.borrow_mut().run, Box::new(|| {}));

        {
            let _tick = traced.then(|| {
                crate::trace::enter(|| crate::trace::Point::Tick {
                    timer: id,
                    name,
                    file: place.file(),
                    line: place.line(),
                })
            });
            run();
        }

        entry.borrow_mut().run = run;
    }

    if waited.is_cancelled() {
        return;
    }
    wait(scope, key, &mut entry.borrow_mut());
}

/// What a [`Period::Following`] does when its value changes: waits anew from
/// now, by what the value says now.
fn follow(scope: Scope, key: usize) {
    if let Some(entry) = scope.held_timer(key, Ticking::entry) {
        wait(scope, key, &mut entry.borrow_mut());
    }
}

#[cfg(all(test, feature = "test-utils"))]
mod tests {
    use super::*;
    use crate::executor::{Installed, install};
    use crate::scope::ScopeTree;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The test's own clock: timers wait on it, and it moves only when told.
    fn clock() -> Installed {
        install(0)
    }

    fn wait(clock: &Installed, ms: u64) {
        clock.advance(Duration::from_millis(ms));
    }

    fn count(counter: &Arc<AtomicUsize>) -> usize {
        counter.load(Ordering::SeqCst)
    }

    #[track_caller]
    fn counting(tree: &ScopeTree, period: impl Into<Period>) -> (Timer, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        let timer = tree.section().repeat(period, move || {
            c.fetch_add(1, Ordering::SeqCst);
        });

        (timer, counter)
    }

    #[track_caller]
    fn every(tree: &ScopeTree, ms: u64) -> (Timer, Arc<AtomicUsize>) {
        counting(tree, Duration::from_millis(ms))
    }

    fn watched(run: impl FnOnce()) -> Vec<Change> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        changes::watch(move |change| sink.borrow_mut().push(change.clone()));

        run();
        changes::stop_watching();

        seen.take()
    }

    #[test]
    fn devtools_hear_when_a_timer_starts_and_when_it_stops() {
        let _clock = clock();
        let tree = ScopeTree::new();
        let mut id = None;

        let seen = watched(|| {
            let (timer, _) = every(&tree, 30);
            id = timer.id();
            tree.remove();
        });

        let id = id.expect("running");
        assert_eq!(seen, [Change::TimerStarted { id }, Change::TimerStopped { id }]);
    }

    #[test]
    fn an_untraced_timer_leaves_devtools_at_once_and_stops_unheard() {
        let _clock = clock();
        let tree = ScopeTree::new();
        let mut id = None;

        let seen = watched(|| {
            let (timer, _) = every(&tree, 30);
            id = timer.id();
            let _timer = timer.untraced();
            tree.remove();
        });

        let id = id.expect("running");
        assert_eq!(seen, [Change::TimerStarted { id }, Change::TimerStopped { id }]);
    }

    #[test]
    fn a_timer_ticks_every_period() {
        let clock = clock();
        let tree = ScopeTree::new();
        let (_timer, counter) = every(&tree, 30);

        wait(&clock, 29);
        assert_eq!(count(&counter), 0, "ticked before its period was up");

        wait(&clock, 1);
        assert_eq!(count(&counter), 1);

        wait(&clock, 30);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_tick_says_whose_it_is() {
        let clock = clock();
        let tree = ScopeTree::new();
        let ticks = Rc::new(RefCell::new(Vec::new()));
        let heard = ticks.clone();
        crate::trace::observe(move |trace| {
            if let crate::trace::Trace::Begin(record) = trace
                && let crate::trace::Point::Tick { .. } = record.point
            {
                heard.borrow_mut().push(record.point.clone());
            }
        });

        let place = Location::caller();
        let (timer, _) = every(&tree, 30);
        let timer = timer.named("flush");
        wait(&clock, 30);
        crate::trace::stop_observing();

        assert_eq!(
            *ticks.borrow(),
            [crate::trace::Point::Tick {
                timer: timer.id().expect("running"),
                name: Some("flush"),
                file: place.file(),
                line: place.line() + 1,
            }]
        );
    }

    #[test]
    fn a_timer_stops_with_its_scope() {
        let clock = clock();
        let tree = ScopeTree::new();
        let (_timer, counter) = every(&tree, 30);

        wait(&clock, 30);
        assert_eq!(count(&counter), 1);
        tree.remove();

        wait(&clock, 90);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn a_timer_gone_with_its_scope_leaves_nothing_waiting() {
        let clock = clock();
        let tree = crate::scope::ScopeTree::new();
        tree.section().repeat(Duration::from_secs(60), || {});
        clock.run_until_parked();
        assert_eq!(clock.stuck(), 1, "its wait is under way");

        tree.remove();
        clock.run_until_parked();

        assert_eq!(clock.stuck(), 0, "the wait outlived the timer it was for");
    }

    #[test]
    fn a_timer_gone_before_its_first_tick_never_ticks() {
        let clock = clock();
        let tree = ScopeTree::new();
        let (_timer, counter) = every(&tree, 200);

        tree.remove();

        wait(&clock, 250);
        assert_eq!(count(&counter), 0);
    }

    #[test]
    fn a_timer_skips_its_ticks_while_its_scope_sleeps() {
        let clock = clock();
        let tree = ScopeTree::new();
        let (_timer, counter) = every(&tree, 30);

        wait(&clock, 30);
        assert_eq!(count(&counter), 1);

        tree.sleep();
        wait(&clock, 90);
        assert_eq!(count(&counter), 1, "ticked while its scope slept");

        tree.wake();
        wait(&clock, 30);
        assert_eq!(count(&counter), 2, "the ticks it slept through were saved up for it");
    }

    #[test]
    #[should_panic(expected = "starting a timer")]
    fn a_timer_for_a_scope_that_is_gone_is_a_mistake() {
        let _clock = clock();
        let tree = ScopeTree::new();
        let section = tree.section();
        tree.remove();

        section.repeat(Duration::from_millis(30), || {});
    }

    type Watcher = (u64, Arc<dyn Fn() + Send + Sync>);

    /// A value set by hand, telling whoever watches it, from the thread that
    /// set it.
    #[derive(Clone, Default)]
    struct Knob(Arc<std::sync::Mutex<(Option<u64>, Vec<Watcher>, u64)>>);

    impl Knob {
        fn set(&self, ms: u64) {
            let watchers: Vec<Watcher> = {
                let mut knob = self.0.lock().unwrap();
                knob.0 = Some(ms);
                knob.1.clone()
            };
            for (_, changed) in watchers {
                changed();
            }
        }

        fn watchers(&self) -> usize {
            self.0.lock().unwrap().1.len()
        }
    }

    struct Unwatch(Knob, u64);

    impl Drop for Unwatch {
        fn drop(&mut self) {
            let id = self.1;
            self.0.0.lock().unwrap().1.retain(|(watcher, _)| *watcher != id);
        }
    }

    impl Changing for Knob {
        type Value = u64;

        fn now(&self) -> Option<u64> {
            self.0.lock().unwrap().0
        }

        fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn std::any::Any> {
            let mut knob = self.0.lock().unwrap();
            knob.2 += 1;
            let id = knob.2;
            knob.1.push((id, Arc::from(changed)));
            Box::new(Unwatch(self.clone(), id))
        }
    }

    fn following(tree: &ScopeTree, knob: &Knob) -> (Timer, Arc<AtomicUsize>) {
        counting(tree, Period::follows(knob.clone(), Duration::from_millis))
    }

    #[test]
    fn a_period_that_follows_a_value_changes_the_moment_the_value_does() {
        let clock = clock();
        let tree = ScopeTree::new();
        let knob = Knob::default();
        knob.set(1_000);
        let (_timer, counter) = following(&tree, &knob);

        wait(&clock, 100);
        knob.set(50);

        wait(&clock, 50);
        assert_eq!(count(&counter), 1, "the second it had left was waited out");

        wait(&clock, 50);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_change_leaves_only_the_new_wait_waiting() {
        let clock = clock();
        let tree = ScopeTree::new();
        let knob = Knob::default();
        knob.set(60_000);
        let (_timer, _counter) = following(&tree, &knob);
        clock.run_until_parked();

        knob.set(50);
        clock.run_until_parked();

        assert_eq!(clock.stuck(), 1, "the minute's wait went on beside the new one");
    }

    #[test]
    fn a_period_with_nothing_to_follow_waits_for_something() {
        let clock = clock();
        let tree = ScopeTree::new();
        let knob = Knob::default();
        let (_timer, counter) = following(&tree, &knob);

        wait(&clock, 10_000);
        assert_eq!(count(&counter), 0, "ticked with no period to tick by");

        knob.set(30);
        wait(&clock, 30);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn what_a_period_follows_is_watched_for_as_long_as_the_timer_runs() {
        let _clock = clock();
        let tree = ScopeTree::new();
        let knob = Knob::default();
        knob.set(1_000);

        let (_timer, _counter) = following(&tree, &knob);
        assert_eq!(knob.watchers(), 1);

        tree.remove();
        assert_eq!(knob.watchers(), 0, "the timer stopped and kept watching");
    }

    #[test]
    fn a_tick_may_list_the_timers_itself() {
        let clock = clock();
        let tree = ScopeTree::new();
        let seen = Arc::new(AtomicUsize::new(0));

        let s = seen.clone();
        let scope = tree.scope();
        tree.section().repeat(Duration::from_millis(30), move || {
            s.store(scope.timers().len(), Ordering::SeqCst);
        });

        wait(&clock, 30);
        assert_eq!(count(&seen), 1);
    }

    #[test]
    fn a_tick_may_start_another_timer() {
        let clock = clock();
        let tree = ScopeTree::new();
        let section = tree.section();

        tree.section().repeat(Duration::from_millis(30), move || {
            section.repeat(Duration::from_secs(60), || {});
        });

        wait(&clock, 30);
        assert_eq!(tree.scope().timers().len(), 2);
    }

    #[test]
    fn a_running_timer_says_where_it_was_set_up_and_what_it_is_called() {
        let _clock = clock();
        let tree = ScopeTree::new();
        let page = tree.child();
        let (timer, _) = every(&tree, 1_000);
        let (hidden, _) = every(&tree, 1_000);
        let below = page.section().repeat(Duration::from_millis(1_000), || {});

        let timer = timer.named("sweep");
        let hidden = hidden.untraced().id();

        let id = timer.id().expect("running");
        let listed = tree.scope().timers();
        let info = listed.iter().find(|info| info.id == id).expect("listed");
        assert_eq!(info.name, Some("sweep"));
        assert!(info.place.file().ends_with("timers.rs"));
        assert_eq!(info.period, Duration::from_millis(1_000));
        assert_eq!(info.scope, tree.scope());
        assert!(listed.iter().all(|info| Some(info.id) != hidden));
        assert!(listed.iter().any(|info| Some(info.id) == below.id()), "a timer below is listed too");
        assert!(tree.scope().timers_here().iter().all(|info| Some(info.id) != below.id()));

        tree.remove();
        assert_eq!(timer.id(), None);
    }
}
