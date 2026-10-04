use std::any::TypeId;

use guinea_core::feature::Exported;
use guinea_core::scope::Scope;

use super::builder::{FeatureBuilder, PluginBuilder};

/// Reusable, application-agnostic: the application knows the plugin, never the
/// other way round.
pub trait Plugin: Send + 'static {
    /// Identity for diagnostics and for installing at most once.
    const ID: &'static str;

    /// What every window may read of what it claims, as a feature's
    /// [`Exports`](crate::feature::Feature::Exports): `()`, or a tuple.
    type Exports: Exported;

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()>;
}

/// This application's own wiring; sees everything a plugin sees and more.
pub trait AppFeature: Send + 'static {
    /// What every window may read of what it claims - see
    /// [`Plugin::Exports`].
    type Exports: Exported;

    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()>;
}

/// Returned from [`Plugin::build`] or [`AppFeature::install`] to end the
/// application before it starts: what is already installed is torn down, no
/// window opens, and `run` returns `Ok`.
///
/// ```ignore
/// if another_copy_is_running() {
///     return Err(Stop.into());
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop;

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the application stopped before it started")
    }
}

impl std::error::Error for Stop {}

pub(crate) type Settle = fn(Scope, &'static str) -> anyhow::Result<()>;

fn settle<U: 'static, E: Exported>(scope: Scope, who: &'static str) -> anyhow::Result<()> {
    if let Some(reducer) = E::unclaimed(scope) {
        anyhow::bail!(
            "`{who}` exports {reducer}, but nothing in the application claimed it - \
             claim it with `app.state::<{reducer}>()`, or take it out of `Exports`"
        );
    }

    scope.mark_feature_installed::<U>();
    E::mark(scope);
    Ok(())
}

pub(crate) trait ErasedPlugin: Send {
    fn id(&self) -> &'static str;
    fn concrete(&self) -> TypeId;
    fn settle(&self) -> Settle;
    fn build_boxed(self: Box<Self>, app: &mut PluginBuilder) -> anyhow::Result<()>;
}

impl<P: Plugin> ErasedPlugin for P {
    fn id(&self) -> &'static str {
        P::ID
    }

    fn concrete(&self) -> TypeId {
        TypeId::of::<P>()
    }

    fn settle(&self) -> Settle {
        settle::<P, P::Exports>
    }

    fn build_boxed(self: Box<Self>, app: &mut PluginBuilder) -> anyhow::Result<()> {
        Plugin::build(*self, app)
    }
}

pub(crate) trait ErasedFeature: Send {
    fn name(&self) -> &'static str;
    fn concrete(&self) -> TypeId;
    fn settle(&self) -> Settle;
    fn install_boxed(self: Box<Self>, app: &mut FeatureBuilder) -> anyhow::Result<()>;
}

impl<F: AppFeature> ErasedFeature for F {
    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn concrete(&self) -> TypeId {
        TypeId::of::<F>()
    }

    fn settle(&self) -> Settle {
        settle::<F, F::Exports>
    }

    fn install_boxed(self: Box<Self>, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        AppFeature::install(*self, app)
    }
}
