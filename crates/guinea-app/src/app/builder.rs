use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

use anyhow::Context as _;
use guinea_core::actor::shape::name;
use guinea_core::actor::{Addr, ManagedActor};
use guinea_core::__private::short_type_name;
use guinea_core::scope::ScopeTree;
use crate::feature::{ScopeContext, publish, settle};

use super::observed::Observed;
use super::plugin::{AppFeature, Plugin};
use super::registry::{Backend, Plugins, Registry, Unit};
use super::windows::WindowService;

/// What a plugin may do while it builds.
///
/// Everything a [`ScopeContext`] may do, in the application's own scope - the
/// one every window sits under - plus cleaning up at exit and reaching the
/// windows the backend opens. What it lets every window read is its
/// [`Exports`](Plugin::Exports); what it hands the application is what
/// [`build`](Plugin::build) returns.
pub struct PluginBuilder {
    cx: ScopeContext,
    /// The application's tree: its scope is the root, the windows are under
    /// it, and letting go of the tree removes all of it - the windows first,
    /// then what the application set up, the last first.
    tree: ScopeTree,
    registry: RefCell<Registry>,
}

/// The application being built: what [`run`](crate) hands the application's
/// function, and what an application feature is installed with.
///
/// Everything a plugin may do, plus installing plugins and features, and
/// providing what windows and features take by type.
pub struct FeatureBuilder {
    inner: PluginBuilder,
}

impl PluginBuilder {
    pub(crate) fn new() -> Self {
        let tree = ScopeTree::new();
        tree.scope().keep(Plugins::default());
        tree.scope().keep(Backend::default());

        Self {
            cx: ScopeContext::new(tree.scope()),
            tree,
            registry: RefCell::default(),
        }
    }

    /// The windows the backend opens, or `None` under one without windows.
    pub fn windows(&self) -> Option<WindowService> {
        self.cx.scope.service::<WindowService>()
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

    /// The application, for the one tool that watches it.
    pub fn observed(&self) -> Observed {
        Observed::of(self.tree.scope())
    }
}

impl FeatureBuilder {
    pub(crate) fn new() -> Self {
        Self {
            inner: PluginBuilder::new(),
        }
    }

    /// Builds `P` with `params` now, and hands it back: what it exports,
    /// feeds and provides.
    ///
    /// Each call is a copy of its own: two stores are two calls. One that
    /// fails publishes nothing and keeps nothing it set up.
    pub fn plugin<P: Plugin>(&mut self, params: P::Params) -> anyhow::Result<P> {
        let who = name::<P>();

        self.registry.borrow_mut().enter(Unit::Plugin(who));
        let section = self.scope.open_section(None, None);
        let inner = self.inner.cx.in_section(section);
        let outer = std::mem::replace(&mut self.inner.cx, inner);

        let outcome = P::build(&mut self.inner, params)
            .and_then(|built| publish::<P::Exports>(self.scope, who).map(|()| built))
            .with_context(|| format!("plugin `{who}` failed to build"));

        self.inner.cx = outer;
        self.registry.borrow_mut().leave();

        match &outcome {
            Ok(_) => Plugins::mark(self.scope, who),
            Err(_) => section.withdraw(),
        }
        outcome
    }

    /// Provides `value` as the one `T` of the application: what every window
    /// and feature taking a `T` is handed, by clone.
    ///
    /// Providing a second `T` is a setup mistake, and panics naming who
    /// provided the first.
    pub fn provide<T: Clone + 'static>(&self, value: T) -> &Self {
        let by = self.registry.borrow().current();

        if let Err(first) = self.scope.provide(value, by) {
            panic!(
                "`{}` is provided by `{first}`, and again by `{by}` - an application has one \
                 of each service",
                short_type_name::<T>()
            );
        }
        self
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

impl crate::feature::given::Context for FeatureBuilder {
    fn lookup(&self) -> crate::feature::given::Lookup {
        crate::feature::given::Lookup(self.inner.cx.scope)
    }
}
