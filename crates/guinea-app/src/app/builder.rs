use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

use anyhow::Context as _;
use guinea_core::actor::shape::name;
use guinea_core::actor::{Addr, ManagedActor};
use guinea_core::scope::{Scope, ScopeTree};
use guinea_core::short_type_name;
use crate::feature::{ScopeContext, settle};

use super::plugin::{AppFeature, ErasedPlugin, Identity, Plugin};
use super::registry::{Admission, Plugins, Registry, Unit};

/// What a plugin may do during installation.
///
/// Everything a [`ScopeContext`] may do, in the application's own scope - the
/// one every window sits under - plus what only the application has:
/// installing other plugins, providing services, cleaning up at exit. What it
/// lets every window read is its [`Exports`](Plugin::Exports).
pub struct PluginBuilder {
    cx: ScopeContext,
    /// The application's tree: its scope is the root, the windows are under
    /// it, and letting go of the tree removes all of it - the windows first,
    /// then what the application set up, the last first.
    tree: ScopeTree,
    registry: RefCell<Registry>,
}

/// Everything a plugin may do, plus this application's own wiring.
pub struct FeatureBuilder {
    inner: PluginBuilder,
}

impl PluginBuilder {
    pub(crate) fn new() -> Self {
        let tree = ScopeTree::new();
        tree.scope().keep(Plugins::default());

        Self {
            cx: ScopeContext::new(tree.scope()),
            tree,
            registry: RefCell::default(),
        }
    }

    pub fn plugin<P: Plugin>(&mut self, plugin: P) -> anyhow::Result<()> {
        self.install_plugin(Identity::plugin::<P>(), Box::new(plugin))
    }

    /// Provides `value` as the one `T` of the application: what every install
    /// function taking a `T` is handed, by clone.
    ///
    /// Providing a second `T` is a setup mistake, and panics naming who
    /// provided the first.
    pub fn provide<T: Clone + 'static>(&self, value: T) -> &Self {
        let by = self.registry.borrow().current();

        if let Err(first) = self.cx.scope.provide(value, by) {
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
        self.tree.on_teardown(move || {
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

        let admitted = self
            .registry
            .borrow()
            .admit_plugin(self.scope, identity.id, identity.concrete)?;
        match admitted {
            Admission::AlreadyInstalled => return Ok(()),
            Admission::Proceed => {}
        }

        self.registry.borrow_mut().enter(Unit::Plugin(identity.id));
        let section = self.scope.open_section(None, None);
        let inner = self.cx.in_section(section);
        let outer = std::mem::replace(&mut self.cx, inner);
        let outcome = plugin
            .build_boxed(self)
            .and_then(|()| (identity.settle)(self.scope, id))
            .with_context(|| format!("plugin `{id}` failed to build"));
        self.cx = outer;

        self.registry.borrow_mut().leave();
        match outcome {
            Ok(()) => Plugins::mark(self.scope, identity.id, identity.concrete),
            Err(_) => section.withdraw(),
        }
        outcome
    }
}

impl FeatureBuilder {
    pub(crate) fn new() -> Self {
        Self {
            inner: PluginBuilder::new(),
        }
    }

    /// Installs `F` in the application's scope, and publishes what it exports
    /// to every window.
    ///
    /// The application has one of each feature: a second install of `F` is
    /// an error naming it. One that fails publishes nothing and keeps nothing
    /// it set up.
    pub fn install<F: AppFeature>(&mut self) -> anyhow::Result<F> {
        let who = name::<F>();
        anyhow::ensure!(
            !self.scope.has_feature::<F>(),
            "`{who}` is already installed in the application - it installs a feature once"
        );

        self.registry.borrow_mut().enter(Unit::Feature(who));
        let section = self.scope.open_section(Some(who), F::DECLARED);
        let inner = self.inner.cx.in_section(section);
        let outer = std::mem::replace(&mut self.inner.cx, inner);

        let outcome = F::install(self)
            .and_then(|installed| settle::<F, F::Exports>(self.scope, who).map(|()| installed))
            .with_context(|| format!("feature `{who}` failed to install"));

        self.inner.cx = outer;
        self.registry.borrow_mut().leave();

        if outcome.is_err() {
            section.withdraw();
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
    fn scope(&self) -> Scope {
        self.cx.scope
    }
}

impl crate::feature::given::Context for FeatureBuilder {
    fn scope(&self) -> Scope {
        self.inner.cx.scope
    }
}
