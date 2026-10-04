use std::rc::Rc;

use guinea_core::SharedState;
use guinea_core::actor::event_bus::EventBus;
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::{Scope, ScopeGuard, ScopeTree};

use super::{FeatureInitContext, ScopeContext};
use crate::app::roots::{Registration, RootId};

/// What a feature needs to be installed into a scope: the window's own scope,
/// the UI thread, the window's event bus, and whatever plugins provided.
///
/// The router used to own all of these and hand them out. It shouldn't: they are
/// no more about routing than they are about anything else, and an
/// application that never navigates - a single window, a dialog, a backend
/// with no notion of a route - needs them just the same. So the host lives
/// here, and the router is one of its callers.
pub struct FeatureHost {
    /// The window's own scope, which every scope installed through this host
    /// sits under, and which sits under the application's. Removed when the
    /// host goes, before its registration does.
    scope: ScopeGuard,
    /// The tree the window's scope is in, when there is no application to
    /// host it - a test, say.
    _tree: Option<ScopeTree>,
    /// The application's own context, when there is one.
    app: Option<ScopeContext>,
    token: UiThreadToken,
    /// One per window, shared by every feature installed through this host,
    /// so actors in different features can reach each other.
    event_bus: Rc<EventBus>,
    services: SharedState,
    /// This host *is* the root, so its registration is held here: the entry
    /// goes when the host goes, together with the scopes under it.
    root: Registration,
}

impl FeatureHost {
    /// A window of `app`: its scope under the application's, with what the
    /// application's plugins provided.
    ///
    /// `app` is the application's own context - what
    /// [`AppRuntime::context`](crate::app::AppRuntime::context) hands back.
    pub fn under(app: &ScopeContext) -> Self {
        Self::open(app.token.clone(), app.scope, None, app.services.clone(), Some(app.clone()))
    }

    /// A window with no application around it - a test, say. Nothing is
    /// provided, and it reads only what its own segments export.
    pub fn detached(token: UiThreadToken) -> Self {
        let tree = ScopeTree::new();
        let parent = tree.scope();
        Self::open(token, parent, Some(tree), SharedState::default(), None)
    }

    fn open(
        token: UiThreadToken,
        parent: Scope,
        tree: Option<ScopeTree>,
        services: SharedState,
        app: Option<ScopeContext>,
    ) -> Self {
        let root = Registration::open();
        let id = root.id().get();
        let scope = parent.child();
        scope.set_window(id);

        Self {
            scope: scope.guard(),
            _tree: tree,
            app,
            token,
            event_bus: Rc::new(EventBus::for_root(id)),
            services,
            root,
        }
    }

    /// The application this window belongs to, when it has one - what a
    /// window opened from this one is opened under.
    pub fn application(&self) -> Option<&ScopeContext> {
        self.app.as_ref()
    }

    /// The window's own scope: the root of every scope installed here.
    pub fn scope(&self) -> Scope {
        self.scope.scope()
    }

    pub fn token(&self) -> &UiThreadToken {
        &self.token
    }

    /// Which root this is - the one a feature installed here belongs to.
    pub fn root(&self) -> RootId {
        self.root.id()
    }

    pub fn event_bus(&self) -> &Rc<EventBus> {
        &self.event_bus
    }

    /// What plugins provided at startup.
    ///
    /// The one thing an enter guard can read: it answers before anything is
    /// installed, so there is no scope yet - only what the application holds.
    pub fn services(&self) -> &SharedState {
        &self.services
    }

    /// The context a feature installs through, for the segment at `cursor`
    /// of its chain.
    pub fn context(&self, scope: Scope, cursor: usize) -> FeatureInitContext {
        FeatureInitContext {
            scope_cx: ScopeContext {
                scope,
                token: self.token.clone(),
                services: self.services.clone(),
            },
            cursor,
            root: self.root.id(),
            event_bus: self.event_bus.clone(),
        }
    }

    /// Installs one feature into a scope of its own, right under the window's,
    /// which goes when the guard does.
    ///
    /// The whole path for an application that has no routes: no chain, no
    /// route, no backend.
    pub fn install(
        &self,
        install: impl Fn(&FeatureInitContext) -> anyhow::Result<()>,
    ) -> anyhow::Result<ScopeGuard> {
        let scope = self.scope().child().guard();
        let ctx = self.context(scope.scope(), 0);
        install(&ctx)?;
        Ok(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_core::actor::{Cx, Handler};
    use guinea_core::observability::changes::{self, Change};
    use guinea_core::trace::Bus;
    use std::cell::RefCell;

    #[derive(Debug)]
    struct Counter;

    struct Bump;

    guinea_macros::actor! {
        Counter {
            handlers {
                Bump
            }
        }
    }

    impl Handler<Bump> for Counter {
        fn handle(&mut self, _: Bump, _cx: Cx<Self, Bump>) {}
    }

    #[derive(Clone)]
    struct Ping;
    impl guinea_core::actor::event_bus::Event for Ping {}

    #[test]
    fn what_changes_under_a_host_names_its_window() {
        let host = FeatureHost::detached(UiThreadToken::dangerously_create_token_unchecked());
        let root = Some(host.root().get());

        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        changes::watch(move |change| sink.borrow_mut().push(change.clone()));

        let scope = host
            .install(|ctx| {
                ctx.spawn(Counter);
                ctx.subscribe(|_: Ping| {});
                Ok(())
            })
            .expect("installed");
        drop(scope);
        changes::stop_watching();

        let seen = seen.take();
        let windows: Vec<(&str, Option<u64>)> = seen
            .iter()
            .filter_map(|change| match change {
                Change::ActorAdded { root, .. } => Some(("actor", *root)),
                Change::Subscriptions { bus: Bus::Window, root } => Some(("bus", *root)),
                _ => None,
            })
            .collect();
        assert_eq!(
            windows,
            [("actor", root), ("bus", root), ("bus", root)],
            "{seen:?}"
        );
    }
}
