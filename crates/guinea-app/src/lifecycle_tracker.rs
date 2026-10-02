use guinea_core::actor::addr::Addr;
use guinea_core::actor::event_bus::subscribe::BusSubscription;
use guinea_core::lifecycle_tracker::LifecycleTracker;
use guinea_core::scope::{DropGuard, Scope, Teardown};
use std::cell::RefCell;
use std::rc::Rc;

/// The application's lifetime: its scope, which holds everything plugins and
/// features set up, and the actors to count once that scope is gone.
#[derive(Clone)]
pub struct AppLifecycle {
    inner: Rc<AppLifecycleInner>,
}

struct AppLifecycleInner {
    scope: Scope,
    counted: RefCell<Vec<Rc<&'static str>>>,
}

struct Cleanup(Box<dyn FnOnce()>);

impl Teardown for Cleanup {
    fn teardown(self) {
        (self.0)();
    }
}

impl Default for AppLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl AppLifecycle {
    pub fn new() -> Self {
        let scope = Scope::root();
        crate::app::actors::set_app_scope(scope);

        Self {
            inner: Rc::new(AppLifecycleInner {
                scope,
                counted: RefCell::new(Vec::new()),
            }),
        }
    }

    /// The application's own scope.
    pub fn scope(&self) -> Scope {
        self.inner.scope
    }

    pub(crate) fn on_cleanup(&self, run: impl FnOnce() + 'static) {
        self.inner.scope.own(Cleanup(Box::new(run)));
    }

    /// Removes the application's scope - everything in it the last first - and
    /// returns the actors still referenced afterwards.
    pub fn shutdown(self) -> Vec<(&'static str, usize)> {
        self.inner.scope.remove();

        let counted = std::mem::take(&mut *self.inner.counted.borrow_mut());
        let leaked: Vec<(&'static str, usize)> = counted
            .into_iter()
            .filter_map(|counter| {
                let held = Rc::strong_count(&counter) - 1;
                (held > 0).then_some((*counter, held))
            })
            .collect();

        for (actor, refs) in &leaked {
            tracing::error!(actor = %actor, refs, "an actor outlived the application");
        }
        leaked
    }

    pub fn track_loop<T: 'static>(&self, handle: T) {
        self.inner.scope.own(DropGuard(handle));
    }

    /// Counts `addr` among the actors that must be gone once the application
    /// is.
    pub fn track_actor<A: 'static>(&self, addr: &Addr<A>) {
        self.inner.counted.borrow_mut().push(addr.strong_count_ptr());
    }

    pub fn own_actor<A: 'static>(&self, addr: &Addr<A>) {
        self.inner.scope.own(addr.clone());
    }

    pub fn track_sub(&self, subscription: BusSubscription) {
        self.inner.scope.own_subscription(subscription);
    }
}

impl LifecycleTracker for AppLifecycle {
    fn track_loop<T: 'static>(&self, handle: T) {
        self.track_loop(handle);
    }
    fn track_actor<A: 'static>(&self, addr: &Addr<A>) {
        self.track_actor(addr);
    }
    fn track_sub(&self, subscription: BusSubscription) {
        self.track_sub(subscription);
    }
    fn own_actor<A: 'static>(&self, addr: &Addr<A>) {
        self.own_actor(addr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_core::actor::UiThreadToken;
    use guinea_core::actor::event_bus::GlobalEventBus;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCheck(Arc<AtomicUsize>);
    impl Drop for DropCheck {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Clone)]
    struct Ping;
    impl guinea_core::actor::event_bus::Event for Ping {}

    #[derive(Debug)]
    struct Probe;

    guinea_macros::actor! {
        Probe {
            handlers { Ping }
        }
    }

    #[guinea_macros::handler]
    fn probe_ping(_this: &mut Probe, _: Ping) {}

    #[test]
    fn an_actor_owned_by_the_lifecycle_is_not_reported_as_leaked() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let lifecycle = AppLifecycle::new();

        {
            let app = crate::app::PluginBuilder::new(token.clone(), lifecycle.clone());
            let _addr = app.spawn(Probe);
        }

        assert!(lifecycle.shutdown().is_empty());
    }

    #[test]
    fn an_address_kept_past_shutdown_is_reported_once() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let lifecycle = AppLifecycle::new();

        let kept = {
            let app = crate::app::PluginBuilder::new(token.clone(), lifecycle.clone());
            app.spawn(Probe)
        };

        let leaked = lifecycle.shutdown();

        assert_eq!(leaked.len(), 1, "one actor, reported once");
        assert!(leaked[0].0.ends_with("Probe"), "got {}", leaked[0].0);
        assert_eq!(leaked[0].1, 1);
        drop(kept);
    }

    #[test]
    fn test_lifecycle_anchors_cleanup() {
        let lifecycle = AppLifecycle::new();
        let counter = Arc::new(AtomicUsize::new(0));

        lifecycle.track_loop(DropCheck(counter.clone()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        lifecycle.shutdown();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn tracked_subscriptions_end_with_the_application() {
        let lifecycle = AppLifecycle::new();
        lifecycle.track_sub(GlobalEventBus::subscribe_fn(|_: Ping| {}));
        lifecycle.track_sub(GlobalEventBus::subscribe_fn(|_: Ping| {}));

        assert_eq!(GlobalEventBus::count_subscribers::<Ping>(), 2);

        lifecycle.clone().shutdown();

        assert_eq!(
            GlobalEventBus::count_subscribers::<Ping>(),
            0,
            "the bus itself must be left empty, not merely the tracker"
        );
    }
}
