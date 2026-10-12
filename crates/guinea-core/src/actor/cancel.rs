//! What ends a background task when the actor that started it is gone.

use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    woken: Notify,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends the work this token stands for, and wakes whoever waits on it.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        self.inner.woken.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
            || self.parent.as_ref().is_some_and(|parent| parent.is_cancelled())
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

        if self.inner.cancelled.load(Ordering::SeqCst) {
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

    #[tokio::test]
    async fn waiting_on_a_token_cancelled_earlier_returns_at_once() {
        let cancel = Cancel::new();
        cancel.cancel();

        cancel.cancelled().await;
        assert!(cancel.guard(async { 7 }).await.is_none());
    }
}
