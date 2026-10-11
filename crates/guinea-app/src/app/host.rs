use guinea_core::scope::{Scope, ScopeTree, Teardown};

/// What hosts the application: the tree of scopes every window sits in.
///
/// The application's own scope is the tree's root. What plugins and features
/// set up is owned by it, the windows are scopes under it, and letting go of
/// the host - or [`shutdown`](Self::shutdown) - removes all of it: the
/// windows first, then what the application set up, the last first.
pub struct AppHost {
    tree: ScopeTree,
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
        Self {
            tree: ScopeTree::new(),
        }
    }

    /// The application's own scope: the root of every window's.
    pub fn scope(&self) -> Scope {
        self.tree.scope()
    }

    pub(crate) fn on_cleanup(&self, run: impl FnOnce() + 'static) {
        self.tree.own(Cleanup(Box::new(run)));
    }

    /// Removes the application's scope: everything in it, the last first.
    pub fn shutdown(&self) {
        self.tree.remove();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use guinea_core::scope::DropGuard;

    use super::*;

    struct DropCheck(Arc<AtomicUsize>);

    impl Drop for DropCheck {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Clone)]
    struct Ping;

    impl guinea_core::actor::event_bus::Event for Ping {}

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
        let bus = host.scope().bus();
        host.scope().own_subscription(bus.subscribe_fn(|_: Ping| {}));
        host.scope().own_subscription(bus.subscribe_fn(|_: Ping| {}));

        assert_eq!(bus.count_subscribers::<Ping>(), 2);

        host.shutdown();

        assert_eq!(
            bus.count_subscribers::<Ping>(),
            0,
            "the bus itself must be left empty, not merely the host"
        );
    }
}
