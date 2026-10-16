//! Where guinea's own background work runs.
//!
//! Everything guinea schedules - a background task, an answer carried back to
//! the UI thread - goes through here rather than straight to tokio, so that a
//! test can put a [`Seeded`] executor on its thread and decide the order
//! itself. Outside such a test this is `tokio::spawn`.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Runs `task` in the background.
pub fn spawn<F>(task: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    #[cfg(feature = "test-utils")]
    let task = match seeded::spawn_here(Box::pin(task)) {
        Ok(()) => return,
        Err(task) => task,
    };

    tokio::spawn(task);
}

/// Gives way once: the task is woken and set aside, and whatever else is
/// ready runs before it resumes.
///
/// Under a [`Seeded`] executor this is where two tasks can change places, so a
/// fake that stands in for slow work yields where the real work would wait.
pub fn yield_now() -> impl Future<Output = ()> {
    YieldNow { yielded: false }
}

/// Gives way a random number of times - as many as the seed says under a
/// [`Seeded`] executor, once outside one.
///
/// What a fake does where the real work takes as long as it takes. The number
/// comes from the executor's own generator, so a seed replays the delays along
/// with the order.
pub fn random_delay() -> impl Future<Output = ()> {
    RandomDelay { left: None }
}

#[cfg(feature = "test-utils")]
const LONGEST_DELAY: usize = 10;

struct RandomDelay {
    left: Option<usize>,
}

impl Future for RandomDelay {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let left = *self.left.get_or_insert_with(draw_delay);
        if left == 0 {
            return Poll::Ready(());
        }

        self.left = Some(left - 1);
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

fn draw_delay() -> usize {
    #[cfg(feature = "test-utils")]
    if let Some(turns) = seeded::draw(LONGEST_DELAY + 1) {
        return turns;
    }

    1
}

struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            return Poll::Ready(());
        }

        self.yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(feature = "test-utils")]
pub use seeded::{Installed, install};

#[cfg(feature = "test-utils")]
pub(crate) use seeded::queue_ui;

#[cfg(feature = "test-utils")]
mod seeded {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::{Rc, Weak};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::Duration;

    use tokio::runtime::Runtime;
    use tokio::time::{Instant, Sleep};

    type Task = Pin<Box<dyn Future<Output = ()> + Send>>;
    type Job = Box<dyn FnOnce() + Send>;

    enum Work {
        Poll(u64),
        Ui(Job),
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// What wakers reach, from whichever thread wakes them.
    ///
    /// Two kinds, kept apart. Tasks that are ready may go in any order - they
    /// run on threads of their own in the application, and whichever finishes
    /// first is what the seed stands for. Jobs for the UI thread may not: the
    /// application's dispatcher queue runs them first in, first out, so two
    /// posts from one place never swap. The seed decides only when the queue
    /// moves, against the tasks.
    #[derive(Default)]
    struct Ready {
        polls: Mutex<Vec<u64>>,
        ui: Mutex<VecDeque<Job>>,
        /// The clock waiting in [`Seeded::advance`] for something to become
        /// ready, if it is.
        waiting: Mutex<Option<Waker>>,
    }

    impl Ready {
        /// Queues `work`. A task woken again before it ran is queued once,
        /// as tokio does.
        fn push(&self, work: Work) {
            match work {
                Work::Poll(id) => {
                    let mut polls = lock(&self.polls);
                    if !polls.contains(&id) {
                        polls.push(id);
                    }
                }
                Work::Ui(job) => lock(&self.ui).push_back(job),
            }

            if let Some(waiting) = lock(&self.waiting).take() {
                waiting.wake();
            }
        }

        fn is_empty(&self) -> bool {
            lock(&self.polls).is_empty() && lock(&self.ui).is_empty()
        }
    }

    /// Lets the clock run until `until`, or until work turns up first - the
    /// answer is whether `until` was reached.
    struct Until<'a> {
        until: Pin<Box<Sleep>>,
        ready: &'a Ready,
    }

    impl Future for Until<'_> {
        type Output = bool;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
            *lock(&self.ready.waiting) = Some(cx.waker().clone());

            if !self.ready.is_empty() {
                return Poll::Ready(false);
            }

            self.until.as_mut().poll(cx).map(|()| true)
        }
    }

    struct Waking {
        task: u64,
        ready: Arc<Ready>,
    }

    impl Wake for Waking {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.ready.push(Work::Poll(self.task));
        }
    }

    thread_local! {
        static CURRENT: RefCell<Option<Rc<Seeded>>> = const { RefCell::new(None) };
    }

    /// One thread, one order: every background task and every answer bound
    /// for the UI thread runs here, and which of the ready ones goes next is
    /// picked by a generator started from `seed`.
    ///
    /// The same seed picks the same order every time, so an order that breaks
    /// something can be had again by its number.
    ///
    /// Its clock is a paused tokio runtime: work runs inside its context, so
    /// `tokio::time::sleep`, `interval` and `timeout` are made against it, and
    /// its time moves only by [`Installed::advance`].
    pub struct Seeded {
        seed: u64,
        state: Cell<u64>,
        ready: Arc<Ready>,
        tasks: RefCell<HashMap<u64, Task>>,
        next: Cell<u64>,
        running: Cell<bool>,
        clock: Runtime,
        /// Where the clock started, which is where tokio's timers count
        /// their milliseconds from.
        origin: Instant,
    }

    const MILLISECOND: Duration = Duration::from_millis(1);

    /// Puts a [`Seeded`] executor on this thread until the guard drops.
    pub fn install(seed: u64) -> Installed {
        let clock = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("a paused runtime to keep the test's time");
        let origin = clock.block_on(async { Instant::now() });

        let executor = Rc::new(Seeded {
            seed,
            state: Cell::new(seed),
            ready: Arc::default(),
            tasks: RefCell::default(),
            next: Cell::new(1),
            running: Cell::new(false),
            clock,
            origin,
        });

        let previous = CURRENT.with(|current| current.borrow_mut().replace(executor.clone()));

        Installed {
            executor,
            previous: previous.as_ref().map_or_else(Weak::new, Rc::downgrade),
        }
    }

    /// The executor on this thread, for as long as it is held. Dropping it
    /// puts back the one it was installed over, if that one is still held.
    pub struct Installed {
        executor: Rc<Seeded>,
        previous: Weak<Seeded>,
    }

    impl Installed {
        pub fn seed(&self) -> u64 {
            self.executor.seed
        }

        /// Runs whatever is ready, one piece at a time in the seed's order,
        /// until nothing is. Time does not move. Returns how many pieces ran.
        pub fn run_until_parked(&self) -> usize {
            self.executor.run_until_parked()
        }

        /// Runs one piece of whatever is ready, the one the seed picks.
        /// `false` when nothing was.
        pub fn step(&self) -> bool {
            self.executor.step()
        }

        /// Moves the clock on by `by`, one timer at a time, running whatever
        /// each timer sets off before the next one fires. Returns how many
        /// pieces ran.
        pub fn advance(&self, by: Duration) -> usize {
            self.executor.advance(by)
        }

        /// Moves the clock to the next timer due within `horizon` and lets it
        /// fire; runs nothing. `false` when none was due - the clock is then
        /// `horizon` further on.
        pub fn wake_next(&self, horizon: Duration) -> bool {
            self.executor.wake_next(horizon)
        }

        /// Tasks that are neither finished nor ready - waiting on a timer the
        /// clock has not reached, or on something nothing here will wake.
        pub fn stuck(&self) -> usize {
            self.executor.stuck()
        }

        /// How far the clock has moved since the executor was installed.
        pub fn elapsed(&self) -> Duration {
            self.executor.now() - self.executor.origin
        }
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            CURRENT.with(|current| {
                let mut current = current.borrow_mut();
                if current.as_ref().is_some_and(|installed| Rc::ptr_eq(installed, &self.executor)) {
                    *current = self.previous.upgrade();
                }
            });
        }
    }

    /// Says the executor is no longer running work when the work is done,
    /// whether it returned or panicked.
    struct Running<'a>(&'a Cell<bool>);

    impl Drop for Running<'_> {
        fn drop(&mut self) {
            self.0.set(false);
        }
    }

    impl Seeded {
        fn spawn(&self, task: Task) {
            let id = self.next.get();
            self.next.set(id + 1);

            self.tasks.borrow_mut().insert(id, task);
            self.ready.push(Work::Poll(id));
        }

        /// SplitMix64: a few lines, no dependency, and a good enough spread
        /// for picking among a handful of tasks.
        fn below(&self, bound: usize) -> usize {
            let mut z = self.state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
            self.state.set(z);

            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;

            (z % bound as u64) as usize
        }

        /// One of the ready tasks, or the job at the head of the UI queue -
        /// which of those, the seed picks.
        fn take(&self) -> Option<Work> {
            let mut polls = lock(&self.ready.polls);
            let mut ui = lock(&self.ready.ui);

            let choices = polls.len() + usize::from(!ui.is_empty());
            if choices == 0 {
                return None;
            }

            let at = self.below(choices);
            if at == polls.len() {
                ui.pop_front().map(Work::Ui)
            } else {
                Some(Work::Poll(polls.swap_remove(at)))
            }
        }

        fn step(&self) -> bool {
            assert!(
                !self.running.replace(true),
                "the executor was asked to run work from inside work it is running"
            );
            let _running = Running(&self.running);

            let _clock = self.clock.enter();

            while let Some(work) = self.take() {
                match work {
                    Work::Ui(job) => {
                        job();
                        return true;
                    }
                    Work::Poll(id) => {
                        if self.poll(id) {
                            return true;
                        }
                    }
                }
            }

            false
        }

        fn run_until_parked(&self) -> usize {
            let mut ran = 0;
            while self.step() {
                ran += 1;
            }
            ran
        }

        fn now(&self) -> Instant {
            self.clock.block_on(async { Instant::now() })
        }

        /// The start of the millisecond `at` falls in, as tokio's timers
        /// count them.
        fn millisecond_of(&self, at: Instant) -> Instant {
            let since = at.saturating_duration_since(self.origin);
            self.origin + Duration::from_millis(since.as_millis() as u64)
        }

        /// Lets the clock run until `target`, or until work turns up first;
        /// whether `target` was reached.
        ///
        /// Never past `target`. Tokio's timers fire on whole milliseconds,
        /// and a paused clock left to itself jumps whole milliseconds from
        /// wherever it is - so from inside a millisecond it would land past
        /// both the timer and the target. The clock is moved exactly into the
        /// next millisecond, left to jump from there to the next timer or the
        /// target's millisecond, and moved exactly through what is left.
        fn run_clock_until(&self, target: Instant) -> bool {
            self.clock.block_on(async {
                loop {
                    let now = Instant::now();
                    if now >= target {
                        return true;
                    }

                    if !self.ready.is_empty() {
                        return false;
                    }

                    let this = self.millisecond_of(now);
                    let next = this + MILLISECOND;
                    if next > target {
                        tokio::time::advance(target - now).await;
                        return true;
                    }

                    if now > this {
                        tokio::time::advance(next - now).await;
                        continue;
                    }

                    let reached = Until {
                        until: Box::pin(tokio::time::sleep_until(self.millisecond_of(target))),
                        ready: &self.ready,
                    }
                    .await;
                    if !reached {
                        return false;
                    }
                }
            })
        }

        fn advance(&self, by: Duration) -> usize {
            let target = self.now() + by;
            let mut ran = self.run_until_parked();

            loop {
                let reached = self.run_clock_until(target);
                ran += self.run_until_parked();

                if reached {
                    return ran;
                }
            }
        }

        fn stuck(&self) -> usize {
            let ready: HashSet<u64> = lock(&self.ready.polls).iter().copied().collect();

            self.tasks
                .borrow()
                .keys()
                .filter(|id| !ready.contains(id))
                .count()
        }

        fn wake_next(&self, horizon: Duration) -> bool {
            let target = self.now() + horizon;
            !self.run_clock_until(target)
        }

        /// Polls task `id`; `false` when there is no such task any more -
        /// it finished after the wake that queued it.
        fn poll(&self, id: u64) -> bool {
            let Some(mut task) = self.tasks.borrow_mut().remove(&id) else {
                return false;
            };

            let waker = Waker::from(Arc::new(Waking {
                task: id,
                ready: self.ready.clone(),
            }));

            if task.as_mut().poll(&mut Context::from_waker(&waker)).is_pending() {
                self.tasks.borrow_mut().insert(id, task);
            }

            true
        }
    }

    fn current() -> Option<Rc<Seeded>> {
        CURRENT.with(|current| current.borrow().clone())
    }

    /// Hands `task` to the executor on this thread, or back if there is none.
    pub(super) fn spawn_here(task: Task) -> Result<(), Task> {
        match current() {
            Some(executor) => {
                executor.spawn(task);
                Ok(())
            }
            None => Err(task),
        }
    }

    /// A number below `bound` from the executor on this thread, if there is
    /// one.
    pub(super) fn draw(bound: usize) -> Option<usize> {
        current().map(|executor| executor.below(bound))
    }

    /// Queues `job` as work for the UI thread, or hands it back if no
    /// executor is on this thread.
    pub(crate) fn queue_ui<F>(job: F) -> Result<(), F>
    where
        F: FnOnce() + Send + 'static,
    {
        match current() {
            Some(executor) => {
                executor.ready.push(Work::Ui(Box::new(job)));
                Ok(())
            }
            None => Err(job),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        fn order(seed: u64) -> Vec<&'static str> {
            let executor = install(seed);
            let seen = Arc::new(Mutex::new(Vec::new()));

            for name in ["a", "b", "c"] {
                let seen = seen.clone();
                crate::executor::spawn(async move {
                    crate::executor::yield_now().await;
                    seen.lock().unwrap().push(name);
                });
            }

            executor.run_until_parked();
            let seen = seen.lock().unwrap().clone();
            seen
        }

        #[test]
        fn one_seed_is_one_order() {
            for seed in 0..16 {
                assert_eq!(order(seed), order(seed), "seed {seed}");
            }
        }

        #[test]
        fn the_seeds_between_them_reach_other_orders() {
            let orders: std::collections::HashSet<Vec<&str>> = (0..64).map(order).collect();
            assert!(orders.len() > 1, "every seed ran the tasks in one order: {orders:?}");
        }

        #[test]
        fn work_for_the_ui_thread_runs_on_this_one() {
            let executor = install(7);
            let ran = Arc::new(AtomicUsize::new(0));

            let counted = ran.clone();
            crate::ui_thread::invoke_on_ui(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });

            executor.run_until_parked();
            assert_eq!(ran.load(Ordering::SeqCst), 1);
        }

        /// Posts to the UI thread from one place arrive in the order they
        /// were made, whatever the seed - the application's dispatcher queue
        /// never swaps them - while tasks ready beside them still move around
        /// them.
        #[test]
        fn posts_to_the_ui_thread_keep_their_order_in_every_seed() {
            let mut interleavings = std::collections::HashSet::new();

            for seed in 0..64 {
                let executor = install(seed);
                let seen = Arc::new(Mutex::new(Vec::new()));

                for post in ["first", "second", "third"] {
                    let seen = seen.clone();
                    crate::ui_thread::invoke_on_ui(move || seen.lock().unwrap().push(post));
                }

                let beside = seen.clone();
                crate::executor::spawn(async move { beside.lock().unwrap().push("task") });

                executor.run_until_parked();
                let seen = seen.lock().unwrap().clone();

                let posts: Vec<&str> = seen.iter().copied().filter(|name| *name != "task").collect();
                assert_eq!(posts, ["first", "second", "third"], "seed {seed}");
                interleavings.insert(seen);
            }

            assert!(interleavings.len() > 1, "the task always ran in one place: {interleavings:?}");
        }

        #[test]
        fn a_sleeping_task_wakes_when_the_clock_reaches_it_and_not_before() {
            let started = std::time::Instant::now();
            let executor = install(3);
            let woke = Arc::new(AtomicUsize::new(0));

            let marked = woke.clone();
            crate::executor::spawn(async move {
                tokio::time::sleep(Duration::from_millis(800)).await;
                marked.fetch_add(1, Ordering::SeqCst);
            });

            executor.run_until_parked();
            assert_eq!(woke.load(Ordering::SeqCst), 0, "time moved without being asked");

            executor.advance(Duration::from_millis(799));
            assert_eq!(woke.load(Ordering::SeqCst), 0, "woke a millisecond early");

            executor.advance(Duration::from_millis(1));
            assert_eq!(woke.load(Ordering::SeqCst), 1);
            assert!(started.elapsed() < Duration::from_millis(200), "waited in real time");
        }

        #[test]
        fn a_timer_that_never_ends_still_lets_advance_return() {
            let executor = install(5);
            let ticks = Arc::new(AtomicUsize::new(0));

            let counted = ticks.clone();
            crate::executor::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_millis(100));
                loop {
                    every.tick().await;
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            });

            executor.advance(Duration::from_secs(1));
            assert_eq!(ticks.load(Ordering::SeqCst), 11, "the first tick and one per 100 ms");
            assert_eq!(executor.stuck(), 1);
        }

        #[test]
        fn work_that_panicked_does_not_leave_the_executor_thinking_it_still_runs() {
            let executor = install(1);
            crate::ui_thread::invoke_on_ui(|| panic!("the job fails"));
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| executor.step()));
            assert!(failed.is_err());

            let ran = Arc::new(AtomicUsize::new(0));
            let counted = ran.clone();
            crate::ui_thread::invoke_on_ui(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });

            let next = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| executor.step()));
            assert!(
                matches!(next, Ok(true)),
                "the next step refused: {:?}",
                next.err().and_then(|panic| panic.downcast_ref::<&str>().map(|text| text.to_string()))
            );
            assert_eq!(ran.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn a_task_its_timer_woke_is_ready_not_stuck() {
            let executor = install(4);
            crate::executor::spawn(async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            });
            executor.run_until_parked();
            assert_eq!(executor.stuck(), 1, "waiting on a timer the clock has not reached");

            assert!(executor.wake_next(Duration::from_secs(1)));
            assert_eq!(executor.stuck(), 0, "its timer fired and it is ready, yet it is counted as stuck");
        }

        #[test]
        fn run_until_parked_counts_only_what_ran() {
            struct WakesTwice(bool);

            impl Future for WakesTwice {
                type Output = ();

                fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                    if self.0 {
                        return Poll::Ready(());
                    }

                    self.0 = true;
                    cx.waker().wake_by_ref();
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }

            let executor = install(2);
            let polls = Arc::new(AtomicUsize::new(0));
            let counted = polls.clone();
            crate::executor::spawn(async move {
                counted.fetch_add(1, Ordering::SeqCst);
                WakesTwice(false).await;
                counted.fetch_add(1, Ordering::SeqCst);
            });

            let ran = executor.run_until_parked();
            assert_eq!(polls.load(Ordering::SeqCst), 2);
            assert_eq!(ran, 2, "two polls ran, but run_until_parked said {ran}");
        }

        #[test]
        fn an_executor_installed_and_dropped_inside_another_leaves_the_outer_one_in_place() {
            let outer = install(1);
            drop(install(2));

            let ran = Arc::new(AtomicUsize::new(0));
            let counted = ran.clone();
            let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::executor::spawn(async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                });
            }));

            assert!(spawned.is_ok(), "spawn found no executor while the outer one is still held");
            outer.run_until_parked();
            assert_eq!(ran.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn advancing_by_less_than_a_millisecond_moves_the_clock_by_that_much() {
            let executor = install(9);
            let woke = Arc::new(AtomicUsize::new(0));

            let marked = woke.clone();
            crate::executor::spawn(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                marked.fetch_add(1, Ordering::SeqCst);
            });
            executor.run_until_parked();

            let started = executor.executor.now();
            for _ in 0..10 {
                executor.advance(Duration::from_micros(100));
            }
            let moved = executor.executor.now() - started;

            assert_eq!(
                woke.load(Ordering::SeqCst),
                0,
                "a 5 ms sleep woke after 1 ms was asked for (clock moved {moved:?})"
            );
            assert_eq!(moved, Duration::from_millis(1), "ten advances of 100 us");
        }

        #[test]
        fn a_task_waiting_on_nothing_is_counted_as_stuck() {
            let executor = install(1);
            crate::executor::spawn(std::future::pending());

            executor.run_until_parked();
            assert_eq!(executor.stuck(), 1);
        }
    }
}
