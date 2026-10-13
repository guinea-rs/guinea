use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use anyhow::Context as _;
use guinea_core::actor::{Addr, ManagedActor};
use guinea_core::{SharedState, short_type_name};
use crate::feature::ScopeContext;
use crate::feature::given::Context as _;

use super::host::AppHost;
use super::plugin::{AppFeature, ErasedFeature, ErasedPlugin, Identity, Installed, Plugin};
use super::registry::{Admission, Registry, Unit};

/// What a plugin may do during installation.
///
/// Everything a [`ScopeContext`] may do, in the application's own scope - the
/// one every window sits under - plus what only the application has:
/// installing other plugins, providing services, cleaning up at exit. What it
/// lets every window read is its [`Exports`](Plugin::Exports).
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
    pub(crate) fn new(host: AppHost) -> Self {
        Self {
            cx: ScopeContext::new(host.scope(), SharedState::new()),
            host,
            registry: Rc::new(RefCell::new(Registry::default())),
        }
    }

    pub(crate) fn host(&self) -> &AppHost {
        &self.host
    }

    pub(crate) fn observed(&self) -> crate::observability::Observed {
        crate::observability::Observed::new(self.cx.scope, self.registry.clone())
    }

    pub fn plugin<P: Plugin>(&mut self, plugin: P) -> anyhow::Result<Installed<P>> {
        self.install_plugin(Identity::plugin::<P>(), Box::new(plugin))?;
        Ok(Installed::new())
    }

    /// [`FeatureBuilder::feature_as`], for a plugin.
    pub fn plugin_as<T, P>(&mut self, stand_in: P) -> anyhow::Result<Installed<T>>
    where
        T: Plugin,
        P: Plugin<Exports = T::Exports>,
    {
        self.install_plugin(Identity::plugin::<T>(), Box::new(stand_in))?;
        Ok(Installed::new())
    }

    /// Provides `value` as the one `T` of the application: what every install
    /// function taking a `T` is handed, by clone.
    ///
    /// Providing a second `T` is a setup mistake, and panics naming who
    /// provided the first.
    pub fn provide<T: Clone + Send + Sync + 'static>(&self, value: T) -> &Self {
        let by = self.registry.borrow().current();

        if let Err(first) = self.cx.provided().provide(value, by) {
            panic!(
                "`{}` is provided by `{first}`, and again by `{by}` - an application has one \
                 of each service",
                short_type_name::<T>()
            );
        }
        self
    }

    /// Runs `f` when the application exits - after what was set up after it,
    /// and before what was set up before it.
    pub fn on_cleanup(&self, f: impl FnOnce() -> anyhow::Result<()> + 'static) -> &Self {
        self.host.on_cleanup(move || {
            if let Err(e) = f() {
                tracing::error!(error = %e, "an application cleanup failed");
            }
        });
        self
    }

    /// Creates `actor` in the application's scope. See
    /// [`ScopeContext::spawn`].
    pub fn spawn<A: ManagedActor + 'static>(&self, actor: A) -> Addr<A> {
        self.cx.spawn(actor)
    }

    fn install_plugin(
        &mut self,
        identity: Identity,
        plugin: Box<dyn ErasedPlugin>,
    ) -> anyhow::Result<()> {
        let id = plugin.id();

        match self.registry.borrow().admit_plugin(identity.id, identity.concrete)? {
            Admission::AlreadyInstalled => return Ok(()),
            Admission::Proceed => {}
        }

        self.registry.borrow_mut().enter(Unit::Plugin(identity.id));
        let inner = self.cx.in_section(self.scope.open_section(None, None));
        let outer = std::mem::replace(&mut self.cx, inner);
        let outcome = plugin
            .build_boxed(self)
            .and_then(|()| (identity.settle)(self.scope, id))
            .with_context(|| format!("plugin `{id}` failed to build"));
        self.cx = outer;

        let mut registry = self.registry.borrow_mut();
        registry.leave();
        if outcome.is_ok() {
            registry.mark_plugin(identity.id, identity.concrete);
        }
        outcome
    }
}

impl FeatureBuilder {
    pub(crate) fn new(host: AppHost) -> Self {
        Self {
            inner: PluginBuilder::new(host),
        }
    }

    pub fn feature<F: AppFeature>(&mut self, feature: F) -> anyhow::Result<Installed<F>> {
        self.install_feature(Identity::feature::<F>(), Box::new(feature))?;
        Ok(Installed::new())
    }

    /// Installs `stand_in` where the application lists `T`: a test's fake,
    /// which exports exactly what `T` does, so every page reads the same
    /// reducers from it.
    ///
    /// It is installed as `T`: the application has `T`, and a plugin or
    /// feature that installs `T` itself finds it already there. The same
    /// holds for [`plugin_as`](PluginBuilder::plugin_as).
    pub fn feature_as<T, F>(&mut self, stand_in: F) -> anyhow::Result<Installed<T>>
    where
        T: AppFeature,
        F: AppFeature<Exports = T::Exports>,
    {
        self.install_feature(Identity::feature::<T>(), Box::new(stand_in))?;
        Ok(Installed::new())
    }

    fn install_feature(
        &mut self,
        identity: Identity,
        feature: Box<dyn ErasedFeature>,
    ) -> anyhow::Result<()> {
        let name = feature.name();

        match self.registry.borrow().admit_feature(identity.concrete, identity.id) {
            Admission::AlreadyInstalled => return Ok(()),
            Admission::Proceed => {}
        }

        self.registry.borrow_mut().enter(Unit::Feature(name));
        let inner = self.inner.cx.in_section(self.scope.open_section(Some(name), None));
        let outer = std::mem::replace(&mut self.inner.cx, inner);
        let outcome = feature
            .install_boxed(self)
            .and_then(|()| (identity.settle)(self.scope, name))
            .with_context(|| format!("feature `{name}` failed to install"));
        self.inner.cx = outer;

        let mut registry = self.registry.borrow_mut();
        registry.leave();
        if outcome.is_ok() {
            registry.mark_feature(identity.concrete);
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

impl crate::feature::given::Context for PluginBuilder {
    fn provided(&self) -> &SharedState {
        self.cx.provided()
    }
}

impl crate::feature::given::Context for FeatureBuilder {
    fn provided(&self) -> &SharedState {
        self.inner.provided()
    }
}
