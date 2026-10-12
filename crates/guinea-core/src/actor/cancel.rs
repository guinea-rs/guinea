//! What ends a background task when the actor that started it is gone.

use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::Notify;

/// A cancellation token, shared by an actor and everything it spawned.
///
/// Clones share one flag: cancelling any of them cancels them all, and a
/// token cancelled once stays cancelled.
#[derive(Clone, Default)]
pub struct Cancel {
    inner: Arc<Inner>,
    parent: Option<Arc<Cancel>>,
}

/// Why a token was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Reason {
    /// Asked to stop: its actor went, or its lane was cancelled.
    Cancelled = 1,
    /// Newer work took its place.
    Superseded = 2,
}

const LIVE: u8 = 0;

#[derive(Default)]
struct Inner {
    /// [`LIVE`], or the [`Reason`] it was cancelled for.
    state: AtomicU8,
    woken: Notify,
}

impl Inner {
    fn reason(&self) -> Option<Reason> {
        match self.state.load(Ordering::SeqCst) {
            LIVE => None,
            1 => Some(Reason::Cancelled),
            _ => Some(Reason::Superseded),
        }
    }
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends the work this token stands for, and wakes whoever waits on it.
    pub fn cancel(&self) {
        self.cancel_because(Reason::Cancelled);
    }

    /// [`Cancel::cancel`], saying why. The first reason given stays.
    pub fn cancel_because(&self, reason: Reason) {
        let state = &self.inner.state;
        let _ = state.compare_exchange(LIVE, reason as u8, Ordering::SeqCst, Ordering::SeqCst);

        self.inner.woken.notify_waiters();
    }

    /// Why the work was ended: its own reason, else the nearest one above;
    /// `None` while it was not.
    pub fn reason(&self) -> Option<Reason> {
        self.inner
            .reason()
            .or_else(|| self.parent.as_ref().and_then(|parent| parent.reason()))
    }

    pub fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }

    /// Waits for [`Cancel::cancel`], here or above, and returns at once if it
    /// already came.
    pub async fn cancelled(&self) {
        match &self.parent {
            None => self.own().await,
            Some(parent) => tokio::select! {
                () = self.own() => {}
                () = Box::pin(parent.cancelled()) => {}
            },
        }
    }

    async fn own(&self) {
        let mut woken = pin!(self.inner.woken.notified());
        woken.as_mut().enable();

        if self.inner.reason().is_some() {
            return;
        }

        woken.await;
    }

    /// A token of its own that this one's cancellation reaches too: cancelling
    /// it ends what it stands for and nothing above.
    pub fn child(&self) -> Self {
        Self {
            inner: Arc::default(),
            parent: Some(Arc::new(self.clone())),
        }
    }

    /// `fut`'s output, or `None` when cancellation came first - in which case
    /// `fut` is dropped where it last awaited.
    pub async fn guard<F: Future>(&self, fut: F) -> Option<F::Output> {
        tokio::select! {
            biased;
            () = self.cancelled() => None,
            out = fut => Some(out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_guarded_future_is_dropped_when_the_token_is_cancelled() {
        let cancel = Cancel::new();
        let cancelling = cancel.clone();

        tokio::spawn(async move { cancelling.cancel() });

        let forever = std::future::pending::<()>();
        assert!(cancel.guard(forever).await.is_none());
    }

    #[tokio::test]
    async fn a_future_that_finishes_first_still_answers() {
        let cancel = Cancel::new();

        assert_eq!(cancel.guard(async { 7 }).await, Some(7));
        assert!(!cancel.is_cancelled());
    }

    #[tokio::test]
    async fn cancelling_a_child_leaves_its_parent_alone() {
        let parent = Cancel::new();
        let child = parent.child();

        child.cancel();

        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
        assert_eq!(parent.guard(async { 7 }).await, Some(7));
    }

    #[tokio::test]
    async fn cancelling_a_parent_ends_its_children_and_wakes_who_waits_on_them() {
        let parent = Cancel::new();
        let child = parent.child();
        let grandchild = child.child();

        let cancelling = parent.clone();
        tokio::spawn(async move { cancelling.cancel() });

        assert!(grandchild.guard(std::future::pending::<()>()).await.is_none());
        assert!(child.is_cancelled());
    }

    #[test]
    fn a_token_says_why_it_was_cancelled() {
        let cancel = Cancel::new();
        assert_eq!(cancel.reason(), None);

        cancel.cancel_because(Reason::Superseded);

        assert_eq!(cancel.reason(), Some(Reason::Superseded));
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn the_first_reason_given_stays() {
        let cancel = Cancel::new();

        cancel.cancel_because(Reason::Superseded);
        cancel.cancel();

        assert_eq!(cancel.reason(), Some(Reason::Superseded));
    }

    #[test]
    fn a_child_says_its_own_reason_before_its_parents() {
        let parent = Cancel::new();
        let replaced = parent.child();
        let left = parent.child();

        replaced.cancel_because(Reason::Superseded);
        parent.cancel();

        assert_eq!(replaced.reason(), Some(Reason::Superseded));
        assert_eq!(left.reason(), Some(Reason::Cancelled), "the parent's, having none of its own");
    }

    #[tokio::test]
    async fn waiting_on_a_token_cancelled_earlier_returns_at_once() {
        let cancel = Cancel::new();
        cancel.cancel();

        cancel.cancelled().await;
        assert!(cancel.guard(async { 7 }).await.is_none());
    }
}
