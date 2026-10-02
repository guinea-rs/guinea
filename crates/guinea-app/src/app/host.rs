use std::cell::RefCell;
use std::rc::Rc;

use guinea_core::actor::addr::Addr;
use guinea_core::scope::{Scope, ScopeTree, Teardown};

/// What hosts the application: the tree of scopes every window sits in, and
/// the actors to count once that tree is gone.
///
/// The application's own scope is the tree's root. What plugins and features
/// set up is owned by it, the windows are scopes under it, and letting go of
/// the host - or [`shutdown`](Self::shutdown) - removes all of it: the
/// windows first, then what the application set up, the last first.
pub struct AppHost {
    tree: ScopeTree,
    counted: RefCell<Vec<Rc<&'static str>>>,
}

struct Cleanup(Box<dyn FnOnce()>);

impl Teardown for Cleanup {
    fn teardown(self) {
        (self.0)();
    }
}

impl Default for AppHost {
    fn default() -> Self {
        Self::new()
    }
}

impl AppHost {
    pub fn new() -> Self {
        let tree = ScopeTree::new();
        super::actors::set_app_scope(tree.scope());

        Self {
            tree,
            counted: RefCell::new(Vec::new()),
        }
    }

    /// The application's own scope: the root of every window's.
    pub fn scope(&self) -> Scope {
        self.tree.scope()
    }

    pub(crate) fn on_cleanup(&self, run: impl FnOnce() + 'static) {
        self.tree.own(Cleanup(Box::new(run)));
    }

    /// Counts `addr` among the actors that must be gone once the application
    /// is.
    pub(crate) fn count<A: 'static>(&self, addr: &Addr<A>) {
        self.counted.borrow_mut().push(addr.strong_count_ptr());
    }

    /// Removes the application's scope - everything in it the last first - and
    /// returns the actors still referenced afterwards.
    pub fn shutdown(&self) -> Vec<(&'static str, usize)> {
        self.tree.remove();

        let counted = std::mem::take(&mut *self.counted.borrow_mut());
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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use guinea_core::actor::UiThreadToken;
    use guinea_core::actor::event_bus::GlobalEventBus;
    use guinea_core::scope::DropGuard;

    use super::*;
    use crate::app::PluginBuilder;

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

    fn plugin_builder() -> PluginBuilder {
        PluginBuilder::new(
            UiThreadToken::dangerously_create_token_unchecked(),
            AppHost::new(),
        )
    }

    #[test]
    fn an_actor_the_application_holds_is_not_reported_as_leaked() {
        let app = plugin_builder();
        app.spawn(Probe);

        assert!(app.host().shutdown().is_empty());
    }

    #[test]
    fn an_address_kept_past_shutdown_is_reported_once() {
        let app = plugin_builder();
        let kept = app.spawn(Probe);

        let leaked = app.host().shutdown();

        assert_eq!(leaked.len(), 1, "one actor, reported once");
        assert!(leaked[0].0.ends_with("Probe"), "got {}", leaked[0].0);
        assert_eq!(leaked[0].1, 1);
        drop(kept);
    }

    #[test]
    fn what_the_application_holds_goes_with_its_shutdown() {
        let host = AppHost::new();
        let counter = Arc::new(AtomicUsize::new(0));

        host.scope().own(DropGuard(DropCheck(counter.clone())));
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        host.shutdown();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_application_let_go_of_without_a_shutdown_is_torn_down_all_the_same() {
        let host = AppHost::new();
        let counter = Arc::new(AtomicUsize::new(0));
        host.scope().own(DropGuard(DropCheck(counter.clone())));

        drop(host);

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn subscriptions_the_application_holds_end_with_it() {
        let host = AppHost::new();
        host.scope()
            .own_subscription(GlobalEventBus::subscribe_fn(|_: Ping| {}));
        host.scope()
            .own_subscription(GlobalEventBus::subscribe_fn(|_: Ping| {}));

        assert_eq!(GlobalEventBus::count_subscribers::<Ping>(), 2);

        host.shutdown();

        assert_eq!(
            GlobalEventBus::count_subscribers::<Ping>(),
            0,
            "the bus itself must be left empty, not merely the host"
        );
    }
}
