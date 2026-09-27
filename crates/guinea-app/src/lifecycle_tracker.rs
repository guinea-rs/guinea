use crate::feature::AppFeatureDeinitContext;
use guinea_core::actor::UiThreadToken;
use guinea_core::actor::addr::Addr;
use guinea_core::actor::event_bus::subscribe::BusSubscription;
use guinea_core::lifecycle_tracker::LifecycleTracker;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct LifecycleCore {
    subs: Vec<BusSubscription>,
    actor_counters: Vec<Rc<&'static str>>,
    owned: Vec<Box<dyn FnOnce()>>,
    anchors: Vec<Box<dyn Any>>,
}

impl LifecycleCore {
    fn track_loop<T: 'static>(&mut self, handle: T) {
        self.anchors.push(Box::new(handle));
    }

    fn track_actor<A: 'static>(&mut self, addr: &Addr<A>) {
        self.actor_counters.push(addr.strong_count_ptr());
    }

    fn own_actor<A: 'static>(&mut self, addr: &Addr<A>) {
        let addr = addr.clone();
        self.owned.push(Box::new(move || addr.dispose()));
    }

    fn track_sub(&mut self, subscription: BusSubscription) {
        self.subs.push(subscription);
    }

    fn shutdown(&mut self) -> Vec<(&'static str, usize)> {
        self.subs.clear();
        for teardown in self.owned.drain(..).rev() {
            teardown();
        }
        let counters = std::mem::take(&mut self.actor_counters);
        self.anchors.clear();

        counters
            .into_iter()
            .filter_map(|counter| {
                let held = Rc::strong_count(&counter) - 1;
                (held > 0).then_some((*counter, held))
            })
            .collect()
    }
}

// --- AppLifecycle ---

#[derive(Clone, Default)]
pub struct AppLifecycle {
    inner: Rc<RefCell<AppLifecycleInner>>,
}

#[derive(Default)]
struct AppLifecycleInner {
    core: LifecycleCore,
    cleanups: Vec<Box<dyn for<'a> FnOnce(&mut AppFeatureDeinitContext<'a>) -> anyhow::Result<()>>>,
}

impl AppLifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_cleanup(
        &self,
        f: impl for<'a> FnOnce(&mut AppFeatureDeinitContext<'a>) -> anyhow::Result<()> + 'static,
    ) {
        self.inner.borrow_mut().cleanups.push(Box::new(f));
    }

    /// Returns the actors still referenced after teardown.
    pub fn shutdown(
        self,
        token: &UiThreadToken,
        ctx: &mut AppFeatureDeinitContext<'_>,
    ) -> Vec<(&'static str, usize)> {
        crate::app::actors::forget_all();
        let mut inner = self.inner.borrow_mut();
        for cleanup in inner.cleanups.drain(..).rev() {
            if let Err(e) = cleanup(ctx) {
                tracing::error!(error = %e, "an application cleanup failed");
            }
        }
        let _ = token;

        let leaked = inner.core.shutdown();
        for (actor, refs) in &leaked {
            tracing::error!(actor = %actor, refs, "an actor outlived the application");
        }
        leaked
    }

    pub fn track_loop<T: 'static>(&self, handle: T) {
        self.inner.borrow_mut().core.track_loop(handle);
    }

    pub fn track_actor<A: 'static>(&self, addr: &Addr<A>) {
        self.inner.borrow_mut().core.track_actor(addr);
    }

    pub fn own_actor<A: 'static>(&self, addr: &Addr<A>) {
        self.inner.borrow_mut().core.own_actor(addr);
    }

    pub fn track_sub(&self, subscription: BusSubscription) {
        self.inner.borrow_mut().core.track_sub(subscription);
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
    use crate::feature::AppFeatureDeinitContext;
    use guinea_core::SharedState;
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

    fn deinit<'a>(token: &UiThreadToken, shared: &'a SharedState) -> AppFeatureDeinitContext<'a> {
        AppFeatureDeinitContext {
            token: token.clone(),
            shared,
        }
    }

    #[test]
    fn an_actor_owned_by_the_lifecycle_is_not_reported_as_leaked() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let shared = SharedState::new();
        let lifecycle = AppLifecycle::new();

        {
            let mut app = crate::app::PluginBuilder::new(token.clone(), lifecycle.clone());
            let _addr = crate::feature::ContextActorExt::spawn(&mut app, Probe);
        }

        let mut ctx = deinit(&token, &shared);
        assert!(lifecycle.shutdown(&token, &mut ctx).is_empty());
    }

    #[test]
    fn an_address_kept_past_shutdown_is_reported_once() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let shared = SharedState::new();
        let lifecycle = AppLifecycle::new();

        let kept = {
            let mut app = crate::app::PluginBuilder::new(token.clone(), lifecycle.clone());
            crate::feature::ContextActorExt::spawn(&mut app, Probe)
        };

        let mut ctx = deinit(&token, &shared);
        let leaked = lifecycle.shutdown(&token, &mut ctx);

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

        let token = UiThreadToken::dangerously_create_token_unchecked();
        let shared = SharedState::new();
        let mut ctx = deinit(&token, &shared);

        lifecycle.shutdown(&token, &mut ctx);

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn tracked_subscriptions_end_with_the_application() {
        let lifecycle = AppLifecycle::new();
        lifecycle.track_sub(GlobalEventBus::subscribe_fn(|_: Ping| {}));
        lifecycle.track_sub(GlobalEventBus::subscribe_fn(|_: Ping| {}));

        assert_eq!(GlobalEventBus::count_subscribers::<Ping>(), 2);

        let token = UiThreadToken::dangerously_create_token_unchecked();
        let shared = SharedState::new();
        let mut ctx = deinit(&token, &shared);

        lifecycle.clone().shutdown(&token, &mut ctx);

        assert_eq!(
            GlobalEventBus::count_subscribers::<Ping>(),
            0,
            "the bus itself must be left empty, not merely the tracker"
        );
    }
}
