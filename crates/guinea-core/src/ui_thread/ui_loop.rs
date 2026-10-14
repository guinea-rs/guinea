use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use super::{UiDispatcher, UiTask, set_ui_dispatcher};

const LOOK_AGAIN: Duration = Duration::from_millis(5);

/// A UI thread for a loop guinea does not own the queue of: a terminal's, a
/// toolkit that draws frame by frame, an example, a tool, a test. Work bound
/// for the UI thread waits here until the thread that installed it runs it.
pub struct UiLoop {
    tasks: mpsc::Receiver<UiTask>,
    /// A wake-up is on its way, and the next one waits for a drain.
    woken: Arc<AtomicBool>,
    /// What a drain runs, kept between frames so a frame does not allocate.
    ready: Cell<Vec<UiTask>>,
}

struct Queue {
    to_ui: Mutex<mpsc::Sender<UiTask>>,
    wake: Option<Box<dyn Fn() + Send + Sync>>,
    woken: Arc<AtomicBool>,
}

impl UiDispatcher for Queue {
    fn dispatch(&self, task: UiTask) {
        if self.to_ui.lock().unwrap().send(task).is_err() {
            return;
        }

        if let Some(wake) = &self.wake
            && !self.woken.swap(true, Ordering::AcqRel)
        {
            wake();
        }
    }
}

impl UiLoop {
    /// Makes this thread the UI thread, and this loop where work bound for it
    /// waits. Work handed over once the loop is dropped is dropped with it.
    pub fn install() -> Self {
        Self::installed(None)
    }

    /// As [`install`](Self::install), for a loop that sleeps until asked:
    /// `wake` is called, on whichever thread hands work over, when work
    /// arrives at a loop not yet woken for it - once for a burst, and again
    /// after the next [`drain`](Self::drain).
    pub fn install_waking(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::installed(Some(Box::new(wake)))
    }

    fn installed(wake: Option<Box<dyn Fn() + Send + Sync>>) -> Self {
        let (to_ui, tasks) = mpsc::channel();
        let woken = Arc::new(AtomicBool::new(false));

        set_ui_dispatcher(Queue {
            to_ui: Mutex::new(to_ui),
            wake,
            woken: woken.clone(),
        });

        Self {
            tasks,
            woken,
            ready: Cell::default(),
        }
    }

    /// Runs what was waiting when it was called, and leaves what that hands
    /// over for the next call: one frame's worth, so a busy actor cannot
    /// starve the screen. Returns how much ran.
    ///
    /// # Panics
    ///
    /// As [`UiLoop::run`].
    pub fn drain(&self) -> usize {
        self.woken.store(false, Ordering::Release);

        let mut ready = self.ready.take();
        loop {
            match self.tasks.try_recv() {
                Ok(task) => ready.push(task),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => replaced(),
            }
        }

        let ran = ready.len();
        for task in ready.drain(..) {
            task();
        }

        self.ready.set(ready);
        ran
    }

    /// Runs what is waiting, and what that hands over in turn, until nothing
    /// is. Returns how much ran.
    ///
    /// # Panics
    ///
    /// When another loop or dispatcher replaced this one: nothing can reach
    /// it any more.
    pub fn run(&self) -> usize {
        let mut ran = 0;
        loop {
            match self.tasks.try_recv() {
                Ok(task) => task(),
                Err(mpsc::TryRecvError::Empty) => return ran,
                Err(mpsc::TryRecvError::Disconnected) => replaced(),
            }
            ran += 1;
        }
    }

    /// Runs work as it is handed over until `done` holds, or `within` has
    /// passed. Whether `done` came to hold.
    ///
    /// `done` is asked after each piece of work and every few milliseconds
    /// besides, so what comes true off the UI thread is seen too.
    ///
    /// # Panics
    ///
    /// As [`UiLoop::run`].
    pub fn run_until(&self, done: impl Fn() -> bool, within: Duration) -> bool {
        let deadline = Instant::now() + within;

        while !done() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }

            match self.tasks.recv_timeout(left.min(LOOK_AGAIN)) {
                Ok(task) => task(),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => replaced(),
            }
        }

        true
    }
}

fn replaced() -> ! {
    panic!(
        "this UiLoop was replaced - another UiLoop::install or set_ui_dispatcher took the UI \
         thread's work, and nothing will reach this one again"
    )
}
