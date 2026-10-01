//! Timers, owned by the context that set them up.
//!
//! An application's timers live as long as the application; a feature's, as
//! long as its scope. There is no timer without an owner: dropping what the
//! context holds stops it. Each remembers where it was set up, which is how
//! devtools tell one from another, and ticks on the UI thread.
//!
//! They wait on `tokio::time`, through the same executor as background work,
//! so they need the runtime `spawn_bg` needs - and under a test harness they
//! wait on the test's clock and tick when it is advanced.

use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::Location;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use guinea_core::actor::invoke_on_ui;
use guinea_core::scope::Awake;

/// How long between ticks.
pub enum Period {
    Fixed(Duration),
    /// Asked again before every tick: for a period the user can change.
    Varying(Box<dyn Fn() -> Duration>),
}

impl Period {
    pub fn varying(period: impl Fn() -> Duration + 'static) -> Self {
        Period::Varying(Box::new(period))
    }

    fn next(&self) -> Duration {
        match self {
            Period::Fixed(period) => *period,
            Period::Varying(period) => period(),
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
    /// The scope that owns it, as `Scope::key` names it; `None` for the
    /// application.
    pub scope: Option<usize>,
    /// The period of the last tick, or of the first one to come.
    pub period: Duration,
}

struct Entry {
    info: TimerInfo,
    period: Period,
    /// Which chain of wake-ups is the live one. Changing the period starts a
    /// new chain, and whatever the old one had already queued is dropped
    /// when it comes due.
    generation: u64,
    active: Option<Box<dyn Fn() -> bool>>,
    /// The owning scope's, when a scope owns it: asleep, it skips its ticks.
    awake: Option<Awake>,
    run: Box<dyn FnMut()>,
    traced: bool,
}

thread_local! {
    static RUNNING: RefCell<HashMap<u64, Weak<RefCell<Entry>>>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// What keeps a timer running; the context that set it up holds it.
pub struct Ticking {
    entry: Rc<RefCell<Entry>>,
}

impl Drop for Ticking {
    fn drop(&mut self) {
        let id = self.entry.borrow().info.id;
        RUNNING.with(|running| running.borrow_mut().remove(&id));
    }
}

impl guinea_core::scope::Teardown for Ticking {
    fn teardown(self) {
        drop(self);
    }
}

/// A timer that was set up, for saying more about it.
///
/// Holding it does not keep the timer running, and dropping it does not stop
/// it: that is up to the context.
#[derive(Clone)]
pub struct Timer {
    entry: Weak<RefCell<Entry>>,
}

impl Timer {
    fn change(self, change: impl FnOnce(&mut Entry)) -> Self {
        if let Some(entry) = self.entry.upgrade() {
            change(&mut entry.borrow_mut());
        }
        self
    }

    /// What devtools call it, instead of where it was set up.
    pub fn named(self, name: &'static str) -> Self {
        self.change(|entry| entry.info.name = Some(name))
    }

    /// Ticks only while `active` says so; the period runs on regardless.
    pub fn when(self, active: impl Fn() -> bool + 'static) -> Self {
        self.change(|entry| entry.active = Some(Box::new(active)))
    }

    /// Changes how often it ticks, from now.
    ///
    /// The wait already under way is cut short rather than waited out, which
    /// is the difference from a [`Period::Varying`] that answers differently
    /// the next time it is asked: an hourly timer told to tick every second
    /// does so within the second, not within the hour.
    pub fn period(self, period: impl Into<Period>) -> Self {
        let Some(entry) = self.entry.upgrade() else {
            return self;
        };

        let (id, generation, next) = {
            let mut entry = entry.borrow_mut();
            entry.period = period.into();
            entry.generation += 1;

            let next = entry.period.next();
            entry.info.period = next;

            (entry.info.id, entry.generation, next)
        };

        wake_after(id, generation, next);
        self
    }

    /// Leaves no trace and does not show up in devtools: for tooling that
    /// watches the application and must not be seen in what it watches.
    pub fn untraced(self) -> Self {
        self.change(|entry| entry.traced = false)
    }

    pub fn id(&self) -> Option<u64> {
        self.entry.upgrade().map(|entry| entry.borrow().info.id)
    }
}

/// Starts a timer: what keeps it running, for the context to hold, and the
/// timer itself, for the caller to name.
pub(crate) fn start(
    place: &'static Location<'static>,
    feature: Option<&'static str>,
    owner: Option<(usize, Awake)>,
    period: Period,
    run: impl FnMut() + 'static,
) -> (Ticking, Timer) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let first = period.next();
    let (scope, awake) = owner.unzip();

    let entry = Rc::new(RefCell::new(Entry {
        info: TimerInfo {
            id,
            name: None,
            place,
            feature,
            scope,
            period: first,
        },
        period,
        generation: 0,
        active: None,
        awake,
        run: Box::new(run),
        traced: true,
    }));

    RUNNING.with(|running| running.borrow_mut().insert(id, Rc::downgrade(&entry)));
    wake_after(id, 0, first);

    let timer = Timer {
        entry: Rc::downgrade(&entry),
    };
    (Ticking { entry }, timer)
}

/// The timers running on this thread that devtools may see, oldest first.
pub fn running() -> Vec<TimerInfo> {
    let mut timers: Vec<TimerInfo> = RUNNING.with(|running| {
        running
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .filter(|entry| entry.borrow().traced)
            .map(|entry| entry.borrow().info.clone())
            .collect()
    });

    timers.sort_by_key(|timer| timer.id);
    timers
}

fn tick(id: u64, generation: u64) {
    let entry = RUNNING.with(|running| running.borrow().get(&id).and_then(Weak::upgrade));
    let Some(entry) = entry else {
        return;
    };

    // Left over from a period that has since changed: the chain it belonged
    // to ended when the new one was armed.
    if entry.borrow().generation != generation {
        return;
    }

    let (active, traced) = {
        let entry = entry.borrow();
        let active = entry.active.as_ref().is_none_or(|active| active())
            && entry.awake.as_ref().is_none_or(Awake::now);
        (active, entry.traced)
    };

    if active {
        let mut run = std::mem::replace(&mut entry.borrow_mut().run, Box::new(|| {}));

        {
            let _tick = traced
                .then(|| guinea_core::trace::enter(|| guinea_core::trace::Point::Tick { timer: id }));
            run();
        }

        entry.borrow_mut().run = run;
    }

    let next = {
        let mut entry = entry.borrow_mut();
        let next = entry.period.next();
        entry.info.period = next;

        next
    };

    // With the generation this tick came from, not the one the entry holds
    // now: a `Timer::period` from inside `run` has already armed its own
    // chain, and this one is over.
    wake_after(id, generation, next);
}

/// Arms one tick: a task that waits `after` on the clock guinea's work runs by
/// and hands the tick to the UI thread - tokio's in an application, the test's
/// own under a harness.
fn wake_after(id: u64, generation: u64, after: Duration) {
    guinea_core::executor::spawn(async move {
        tokio::time::sleep(after).await;
        invoke_on_ui(move || tick(id, generation));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_core::executor::{Installed, install};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

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

    fn counting(period: u64) -> (Ticking, Timer, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        let (ticking, timer) = start(
            Location::caller(),
            None,
            None,
            Duration::from_millis(period).into(),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );

        (ticking, timer, counter)
    }

    #[test]
    fn a_timer_ticks_every_period() {
        let clock = clock();
        let (_ticking, _timer, counter) = counting(30);

        wait(&clock, 29);
        assert_eq!(count(&counter), 0, "ticked before its period was up");

        wait(&clock, 1);
        assert_eq!(count(&counter), 1);

        wait(&clock, 30);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_timer_stops_with_what_keeps_it() {
        let clock = clock();
        let (ticking, _timer, counter) = counting(30);

        wait(&clock, 30);
        assert_eq!(count(&counter), 1);
        drop(ticking);

        wait(&clock, 90);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn a_timer_dropped_before_its_first_tick_never_ticks() {
        let clock = clock();
        let (ticking, _timer, counter) = counting(200);

        drop(ticking);

        wait(&clock, 250);
        assert_eq!(count(&counter), 0);
    }

    #[test]
    fn changing_the_period_cuts_the_wait_short_and_ends_the_old_chain() {
        let clock = clock();
        let (_ticking, timer, counter) = counting(60_000);

        let timer = timer.period(Duration::from_millis(20));
        wait(&clock, 20);
        assert_eq!(count(&counter), 1, "a minute's wait was waited out");

        let _timer = timer.period(Duration::from_secs(60));
        wait(&clock, 80);
        assert_eq!(
            count(&counter),
            1,
            "the twenty-millisecond chain kept ticking after the period changed"
        );

        wait(&clock, 60_000);
        assert_eq!(count(&counter), 2);
    }

    #[test]
    fn a_timer_ticks_only_while_active() {
        let clock = clock();
        let (_ticking, timer, counter) = counting(30);

        let active = Arc::new(AtomicBool::new(false));
        let a = active.clone();
        let _timer = timer.when(move || a.load(Ordering::SeqCst));

        wait(&clock, 60);
        assert_eq!(count(&counter), 0);

        active.store(true, Ordering::SeqCst);
        wait(&clock, 30);
        assert_eq!(count(&counter), 1);

        active.store(false, Ordering::SeqCst);
        wait(&clock, 60);
        assert_eq!(count(&counter), 1);
    }

    #[test]
    fn a_timer_skips_its_ticks_while_its_scope_sleeps() {
        let clock = clock();
        let scope = guinea_core::scope::Scope::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        let (_ticking, _timer) = start(
            Location::caller(),
            None,
            Some((0, scope.awake())),
            Duration::from_millis(30).into(),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );

        wait(&clock, 30);
        assert_eq!(count(&counter), 1);

        scope.sleep();
        wait(&clock, 90);
        assert_eq!(count(&counter), 1, "ticked while its scope slept");

        scope.wake();
        wait(&clock, 30);
        assert_eq!(
            count(&counter),
            2,
            "the ticks it slept through were saved up for it"
        );
    }

    #[test]
    fn a_tick_may_list_the_running_timers_itself() {
        let clock = clock();
        let seen = Arc::new(AtomicUsize::new(0));

        let s = seen.clone();
        let (_ticking, _timer) = start(
            Location::caller(),
            None,
            None,
            Duration::from_millis(30).into(),
            move || {
                s.store(running().len(), Ordering::SeqCst);
            },
        );

        wait(&clock, 30);
        assert_eq!(count(&seen), 1);
    }

    #[test]
    fn a_running_timer_says_where_it_was_set_up_and_what_it_is_called() {
        let _clock = clock();
        let (ticking, timer, _counter) = counting(1_000);
        let (_kept, hidden, _) = counting(1_000);

        let timer = timer.named("sweep");
        let hidden = hidden.untraced().id();

        let id = timer.id().expect("running");
        let listed = running();
        let info = listed.iter().find(|info| info.id == id).expect("listed");
        assert_eq!(info.name, Some("sweep"));
        assert!(info.place.file().ends_with("timers.rs"));
        assert_eq!(info.period, Duration::from_millis(1_000));
        assert!(listed.iter().all(|info| Some(info.id) != hidden));

        drop(ticking);
        assert!(running().iter().all(|info| info.id != id));
        assert_eq!(timer.id(), None);
    }
}
