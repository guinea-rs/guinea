//! Lending the terminal to someone else for a while: an `ssh`, an editor, a
//! pager - a child process that wants the screen, raw mode off and the keys
//! to itself.
//!
//! The job never runs where it was asked for. It is queued, and the loop in
//! [`crate::run`] runs it between two frames, with the screen released and
//! nothing polling the input; then takes the screen back, redraws it whole
//! and drops whatever was typed at it before the job began.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Runs `job` with the terminal lent to it, between two frames of the running
/// [`run`](crate::run) loop.
///
/// Callable from anywhere: a page's [`on_key`](crate::Page::on_key), an
/// actor, another thread. What the job returns arrives through the [`Lent`];
/// a job that panics resolves it to an error, and the screen is taken back
/// all the same.
pub fn lend_terminal<T, F>(job: F) -> Lent<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let desk = DESK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match desk.as_ref() {
        Some((_, leases)) => lend(leases, job),
        None => Lent::failed(anyhow::anyhow!(
            "lend_terminal() with no terminal: run() is not running"
        )),
    }
}

/// What a lent terminal's job returned, once the loop has run it.
pub struct Lent<T> {
    slot: Arc<Mutex<Slot<T>>>,
}

struct Slot<T> {
    done: Option<anyhow::Result<T>>,
    waker: Option<Waker>,
}

impl<T> Lent<T> {
    fn failed(error: anyhow::Error) -> Self {
        Self {
            slot: Arc::new(Mutex::new(Slot {
                done: Some(Err(error)),
                waker: None,
            })),
        }
    }
}

impl<T> Future for Lent<T> {
    type Output = anyhow::Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        match slot.done.take() {
            Some(done) => Poll::Ready(done),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The other end of a [`Lent`]: kept with what the job returned, or, dropped
/// unkept, with the reason it never ran.
struct Promise<T>(Option<Arc<Mutex<Slot<T>>>>);

impl<T> Promise<T> {
    fn keep(mut self, done: anyhow::Result<T>) {
        if let Some(slot) = self.0.take() {
            settle(&slot, done);
        }
    }
}

impl<T> Drop for Promise<T> {
    fn drop(&mut self) {
        if let Some(slot) = self.0.take() {
            settle(
                &slot,
                Err(anyhow::anyhow!(
                    "the terminal was never lent: the loop ended, or could not let the screen go"
                )),
            );
        }
    }
}

fn settle<T>(slot: &Mutex<Slot<T>>, done: anyhow::Result<T>) {
    let mut slot = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.done = Some(done);

    if let Some(waker) = slot.waker.take() {
        waker.wake();
    }
}

trait Lease: Send {
    fn run(self: Box<Self>);
}

struct Job<T, F> {
    job: F,
    promise: Promise<T>,
}

impl<T, F> Lease for Job<T, F>
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    fn run(self: Box<Self>) {
        let Job { job, promise } = *self;

        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).map_err(|panic| {
            let said = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string()))
                .unwrap_or_default();
            anyhow::anyhow!("the job the terminal was lent to panicked: {said}")
        });

        promise.keep(done);
    }
}

type Leases = Sender<Box<dyn Lease>>;

static DESK: Mutex<Option<(u64, Leases)>> = Mutex::new(None);

fn lend<T, F>(leases: &Leases, job: F) -> Lent<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let slot = Arc::new(Mutex::new(Slot {
        done: None,
        waker: None,
    }));

    let _ = leases.send(Box::new(Job {
        job,
        promise: Promise(Some(slot.clone())),
    }));

    Lent { slot }
}

/// The terminal, as far as a lease needs it.
pub(crate) trait Hand {
    fn release(&mut self) -> io::Result<()>;
    fn take_back(&mut self) -> io::Result<()>;
}

/// Where the leases asked for wait for the loop.
pub(crate) struct Desk {
    id: u64,
    leases: Leases,
    asked: Receiver<Box<dyn Lease>>,
}

impl Desk {
    fn new() -> Self {
        static OPENED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let (leases, asked) = channel();
        Self {
            id: OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            leases,
            asked,
        }
    }

    /// Opens the desk [`lend_terminal`] queues at, for as long as this lives.
    pub(crate) fn open() -> Self {
        let desk = Self::new();
        *DESK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((desk.id, desk.leases.clone()));
        desk
    }

    /// Runs every lease asked for since the last call, with the terminal
    /// released around them.
    ///
    /// A lease that cannot be run because the screen would not let go is
    /// dropped, which resolves it to an error; the screen that would not come
    /// back is the loop's error.
    pub(crate) fn hand_over(&self, hand: &mut impl Hand) -> io::Result<()> {
        let asked: Vec<Box<dyn Lease>> = self.asked.try_iter().collect();
        if asked.is_empty() {
            return Ok(());
        }

        hand.release()?;
        for lease in asked {
            lease.run();
        }
        hand.take_back()
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        let mut desk = DESK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if desk.as_ref().is_some_and(|(open, _)| *open == self.id) {
            *desk = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<&'static str>>>);

    impl Log {
        fn push(&self, what: &'static str) {
            self.0.lock().unwrap().push(what);
        }

        fn taken(&self) -> Vec<&'static str> {
            self.0.lock().unwrap().clone()
        }
    }

    struct Fake(Log);

    impl Hand for Fake {
        fn release(&mut self) -> io::Result<()> {
            self.0.push("release");
            Ok(())
        }

        fn take_back(&mut self) -> io::Result<()> {
            self.0.push("take back");
            Ok(())
        }
    }

    fn now<T>(lent: &mut Lent<T>) -> Option<anyhow::Result<T>> {
        let mut cx = Context::from_waker(Waker::noop());
        match Pin::new(lent).poll(&mut cx) {
            Poll::Ready(done) => Some(done),
            Poll::Pending => None,
        }
    }

    #[test]
    fn the_job_runs_between_frames_with_the_terminal_released() {
        let log = Log::default();
        let desk = Desk::new();

        let mut lent = lend(&desk.leases, {
            let log = log.clone();
            move || {
                log.push("job");
                7
            }
        });

        assert_eq!(log.taken(), Vec::<&str>::new());
        assert!(now(&mut lent).is_none());

        desk.hand_over(&mut Fake(log.clone())).unwrap();

        assert_eq!(log.taken(), ["release", "job", "take back"]);
        assert_eq!(now(&mut lent).map(|done| done.ok()), Some(Some(7)));
    }

    #[test]
    fn a_job_that_panics_is_an_error_and_the_terminal_comes_back() {
        let log = Log::default();
        let desk = Desk::new();

        let mut lent = lend(&desk.leases, || -> u32 { panic!("ssh fell over") });
        desk.hand_over(&mut Fake(log.clone())).unwrap();

        assert_eq!(log.taken(), ["release", "take back"]);
        let said = now(&mut lent).map(|done| done.map_err(|error| error.to_string()));
        assert!(
            matches!(&said, Some(Err(error)) if error.contains("ssh fell over")),
            "{said:?}"
        );
    }

    #[test]
    fn nothing_lent_releases_nothing() {
        let log = Log::default();

        Desk::new().hand_over(&mut Fake(log.clone())).unwrap();

        assert_eq!(log.taken(), Vec::<&str>::new());
    }

    #[test]
    fn a_lease_the_loop_never_ran_is_an_error() {
        let desk = Desk::new();
        let mut lent = lend(&desk.leases, || 7);

        drop(desk);

        assert!(matches!(now(&mut lent), Some(Err(_))));
    }
}
