use guinea_core::scope::{Scope, ScopeGuard, ScopeTree};

use super::{FeatureInitContext, ScopeContext};
use crate::app::roots::{Registration, RootId};

/// What a feature needs to be installed into a scope: the window's own scope,
/// under the application's and what its plugins provided.
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
        Self::open(app.scope, None, Some(app.clone()))
    }

    /// A window with no application around it - a test, say. Nothing is
    /// provided, and it reads only what its own segments export.
    pub fn detached() -> Self {
        let tree = ScopeTree::new();
        let parent = tree.scope();
        Self::open(parent, Some(tree), None)
    }

    fn open(parent: Scope, tree: Option<ScopeTree>, app: Option<ScopeContext>) -> Self {
        let scope = parent.child();
        let root = Registration::open(scope);

        Self {
            scope: scope.guard(),
            _tree: tree,
            app,
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

    /// Which root this is - the one a feature installed here belongs to.
    pub fn root(&self) -> RootId {
        self.root.id()
    }

    /// The context a feature installs through, for the segment at `cursor`
    /// of its chain.
    pub fn context(&self, scope: Scope, cursor: usize) -> FeatureInitContext {
        FeatureInitContext {
            scope_cx: ScopeContext::new(scope),
            cursor,
            root: self.root.id(),
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
    use std::cell::RefCell;
    use std::rc::Rc;

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
    fn an_actor_added_under_a_host_is_in_its_root() {
        let host = FeatureHost::detached();
        let root = Some(host.root());

        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        changes::watch(move |change| {
            if let Change::ActorAdded { owner, .. } = change {
                sink.borrow_mut().push(owner.scope.and_then(crate::app::roots::of));
            }
        });

        let scope = host
            .install(|ctx| {
                ctx.spawn(Counter);
                ctx.subscribe(|_: Ping| {});
                Ok(())
            })
            .expect("installed");
        drop(scope);
        changes::stop_watching();

        assert_eq!(seen.take(), [root]);
    }
}
