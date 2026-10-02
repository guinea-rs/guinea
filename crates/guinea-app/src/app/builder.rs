use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context as _;
use guinea_core::SharedState;
use guinea_core::actor::{Addr, ManagedActor, UiThreadToken};
use guinea_core::scope::Reducer;

use crate::feature::{AppFeatureDeinitContext, ScopeContext};

use super::host::AppHost;
use super::plugin::{AppFeature, ErasedFeature, ErasedPlugin, Plugin};
use super::registry::{Admission, Registry, Unit};

/// What a plugin may do during installation.
///
/// Everything a [`ScopeContext`] may do, in the application's own scope - the
/// one every window sits under - plus what only the application has:
/// installing other plugins, providing services, exporting state to every
/// window, cleaning up at exit.
pub struct PluginBuilder {
    cx: ScopeContext,
    host: AppHost,
    registry: Rc<RefCell<Registry>>,
}

/// Everything a plugin may do, plus this application's own wiring.
pub struct FeatureBuilder {
    inner: PluginBuilder,
}

impl PluginBuilder {
    pub(crate) fn new(token: UiThreadToken, host: AppHost) -> Self {
        Self {
            cx: ScopeContext {
                scope: host.scope(),
                token,
                services: SharedState::new(),
            },
            host,
            registry: Rc::new(RefCell::new(Registry::default())),
        }
    }

    pub(crate) fn host(&self) -> &AppHost {
        &self.host
    }

    pub(crate) fn plugin_ids(&self) -> Vec<&'static str> {
        self.registry.borrow().plugin_ids()
    }

    pub fn plugin<P: Plugin>(&mut self, plugin: P) -> anyhow::Result<&mut Self> {
        self.install_plugin(Box::new(plugin))?;
        Ok(self)
    }

    pub fn provide<T: Send + Sync + 'static>(&self, value: T) -> &Self {
        self.provide_arc(Arc::new(value))
    }

    pub fn provide_arc<T: Send + Sync + 'static>(&self, value: Arc<T>) -> &Self {
        if self.services.insert_arc(value).is_some() {
            tracing::warn!(
                service = std::any::type_name::<T>(),
                by = self.registry.borrow().current(),
                "service replaced"
            );
        }
        self
    }

    pub fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        let service = std::any::type_name::<T>();
        let who = self.registry.borrow().current();

        match self.services.try_get::<T>() {
            Ok(Some(value)) => Ok(value),
            Ok(None) => anyhow::bail!(
                "`{who}` requires service `{service}`, but nothing provided it - \
                 install the plugin that provides it first, usually by calling \
                 `b.plugin(..)?` at the top of `{who}`"
            ),
            Err(poisoned) => Err(anyhow::Error::new(poisoned))
                .with_context(|| format!("`{who}` requires service `{service}`")),
        }
    }

    pub fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services.get::<T>()
    }

    /// Lets every window read `R`, which this plugin or feature claimed with
    /// [`state`](ScopeContext::state): a page reads it as it reads what a
    /// layout above it exports.
    pub fn export<R: Reducer>(&self) -> anyhow::Result<&Self> {
        let reducer = std::any::type_name::<R>();
        anyhow::ensure!(
            self.scope.claims::<R>(),
            "`{}` exports {reducer}, but nothing in the application claimed it - claim it \
             with `app.state::<{reducer}>()` first",
            self.registry.borrow().current()
        );

        self.scope.note_export::<R>();
        Ok(self)
    }

    /// Runs `f` when the application exits - after what was set up after it,
    /// and before what was set up before it.
    pub fn on_cleanup(
        &self,
        f: impl for<'a> FnOnce(&mut AppFeatureDeinitContext<'a>) -> anyhow::Result<()> + 'static,
    ) -> &Self {
        let token = self.token.clone();
        let services = self.services.clone();
        self.host.on_cleanup(move || {
            let mut ctx = AppFeatureDeinitContext {
                token,
                shared: &services,
            };
            if let Err(e) = f(&mut ctx) {
                tracing::error!(error = %e, "an application cleanup failed");
            }
        });
        self
    }

    /// Creates `actor` in the application's scope. See
    /// [`ScopeContext::spawn`]; an application's actor is also counted, and
    /// reported if anything still holds it once the application is gone.
    pub fn spawn<A: ManagedActor + std::fmt::Debug + 'static>(&self, actor: A) -> Addr<A> {
        let addr = self.cx.spawn(actor);
        self.host.count(&addr);
        addr
    }

    fn install_plugin(&mut self, plugin: Box<dyn ErasedPlugin>) -> anyhow::Result<()> {
        let (id, concrete) = (plugin.id(), plugin.concrete());

        match self.registry.borrow().admit_plugin(id, concrete)? {
            Admission::AlreadyInstalled => return Ok(()),
            Admission::Proceed => {}
        }

        self.registry.borrow_mut().enter(Unit::Plugin(id));
        self.scope.open_section(None, None);
        let outcome = plugin
            .build_boxed(self)
            .with_context(|| format!("plugin `{id}` failed to build"));
        self.scope.close_section();

        let mut registry = self.registry.borrow_mut();
        registry.leave();
        if outcome.is_ok() {
            registry.mark_plugin(id, concrete);
        }
        outcome
    }
}

impl FeatureBuilder {
    pub(crate) fn new(token: UiThreadToken, host: AppHost) -> Self {
        Self {
            inner: PluginBuilder::new(token, host),
        }
    }

    pub fn feature<F: AppFeature>(&mut self, feature: F) -> anyhow::Result<&mut Self> {
        self.install_feature(Box::new(feature))?;
        Ok(self)
    }

    fn install_feature(&mut self, feature: Box<dyn ErasedFeature>) -> anyhow::Result<()> {
        let (name, concrete) = (feature.name(), feature.concrete());

        match self.registry.borrow().admit_feature(concrete, name) {
            Admission::AlreadyInstalled => return Ok(()),
            Admission::Proceed => {}
        }

        self.registry.borrow_mut().enter(Unit::Feature(name));
        self.scope.open_section(Some(name), None);
        let outcome = feature
            .install_boxed(self)
            .with_context(|| format!("feature `{name}` failed to install"));
        self.scope.close_section();

        let mut registry = self.registry.borrow_mut();
        registry.leave();
        if outcome.is_ok() {
            registry.mark_feature(concrete);
        }
        outcome
    }
}

impl Deref for FeatureBuilder {
    type Target = PluginBuilder;

    fn deref(&self) -> &PluginBuilder {
        &self.inner
    }
}

impl DerefMut for FeatureBuilder {
    fn deref_mut(&mut self) -> &mut PluginBuilder {
        &mut self.inner
    }
}

impl Deref for PluginBuilder {
    type Target = ScopeContext;

    fn deref(&self) -> &ScopeContext {
        &self.cx
    }
}
