use std::rc::Rc;

use guinea_core::SharedState;
use guinea_core::actor::event_bus::EventBus;
use guinea_core::actor::registry::DebugRegistry;
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::Scope;

use super::FeatureInitContext;
use crate::app::roots::{Registration, RootId};

/// What a feature needs to be installed into a scope: the UI thread, the
/// window's event bus, its debug registry, and whatever plugins provided.
///
/// The router used to own all four and hand them out. It shouldn't: they are
/// no more about routing than they are about anything else, and an
/// application that never navigates - a single window, a dialog, a backend
/// with no notion of a route - needs them just the same. So the host lives
/// here, and the router is one of its callers.
pub struct FeatureHost {
    token: UiThreadToken,
    /// One per window, shared by every feature installed through this host,
    /// so actors in different features can reach each other.
    event_bus: Rc<EventBus>,
    debug_registry: Rc<DebugRegistry>,
    services: SharedState,
    /// This host *is* the root, so its registration is held here: the entry
    /// goes when the host goes, together with the scopes under it.
    root: Registration,
}

impl FeatureHost {
    /// Takes the services from the installed application, or none if there is
    /// no application - a test, say.
    pub fn new(token: UiThreadToken) -> Self {
        Self::with_services(token, crate::app::app_services())
    }

    /// With `services` rather than the installed application's - for a
    /// harness, whose application is its own.
    pub fn with_services(token: UiThreadToken, services: SharedState) -> Self {
        Self {
            token,
            event_bus: Rc::new(EventBus::new()),
            debug_registry: Rc::new(DebugRegistry::new()),
            services,
            root: Registration::open(),
        }
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

    pub fn debug_registry(&self) -> &Rc<DebugRegistry> {
        &self.debug_registry
    }

    /// What plugins provided at startup.
    ///
    /// The one thing an enter guard can read: it answers before anything is
    /// installed, so there is no scope yet - only what the application holds.
    pub fn services(&self) -> &SharedState {
        &self.services
    }

    /// The context a feature installs through. `ancestors` is what
    /// `FeatureInitContext::inherit` walks - root first, never including
    /// `scope` itself.
    pub fn context(&self, scope: Rc<Scope>, ancestors: Rc<[Rc<Scope>]>) -> FeatureInitContext {
        FeatureInitContext {
            scope,
            ancestors,
            root: self.root.id(),
            token: self.token.clone(),
            event_bus: self.event_bus.clone(),
            debug_registry: self.debug_registry.clone(),
            services: self.services.clone(),
        }
    }

    /// Installs one feature into a scope of its own, with nothing above it.
    ///
    /// The whole path for an application that has no routes: no chain, no
    /// `AppUri`, no backend.
    pub fn install(
        &self,
        install: impl Fn(&FeatureInitContext) -> anyhow::Result<()>,
    ) -> anyhow::Result<Rc<Scope>> {
        let scope = Rc::new(Scope::new());
        let ctx = self.context(scope.clone(), Rc::from(Vec::new()));
        install(&ctx)?;
        Ok(scope)
    }
}
