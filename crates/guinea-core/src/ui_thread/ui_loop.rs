use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use super::{UiDispatcher, UiTask, set_ui_dispatcher};

const LOOK_AGAIN: Duration = Duration::from_millis(5);

/// A UI thread for code that has no toolkit to give it one: an example, a
/// tool, a test. Work bound for the UI thread waits here until the thread
/// that installed it runs it.
pub struct UiLoop {
    tasks: mpsc::Receiver<UiTask>,
}

struct Queue(Mutex<mpsc::Sender<UiTask>>);

impl UiDispatcher for Queue {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        let _ = self.0.lock().unwrap().send(task);
    }
}

impl UiLoop {
    /// Makes this thread the UI thread, and this loop where work bound for it
    /// waits. Work handed over once the loop is dropped is dropped with it.
    pub fn install() -> Self {
        let (to_ui, tasks) = mpsc::channel();
        set_ui_dispatcher(Queue(Mutex::new(to_ui)));

        Self { tasks }
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
