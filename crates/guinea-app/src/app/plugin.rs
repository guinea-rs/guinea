use std::any::TypeId;

use guinea_core::actor::shape::Declared;
use guinea_core::feature::Exported;
use guinea_core::scope::Scope;

use super::builder::{FeatureBuilder, PluginBuilder};
use crate::feature::settle;

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

/// This application's own wiring: a feature installed in the application's
/// scope, which every window sits under.
///
/// Written as a feature in a window is, by `feature!` and `#[installs]`, with
/// the builder as the context:
///
/// ```ignore
/// feature! {
///     pub Activity {
///         exports { ActivityState }
///     }
/// }
///
/// #[installs]
/// fn activity(app: &mut FeatureBuilder) -> anyhow::Result<Activity> {
///     let (state, _) = app.state::<ActivityState>().driven_by(ActivityActor::new);
///     Ok(Activity(state))
/// }
/// ```
pub trait AppFeature: Sized + 'static {
    /// What every window may read: what `feature!` lists.
    type Exports: Exported;

    /// Where the feature was written. `#[installs]` fills it in.
    const DECLARED: Option<Declared> = None;

    fn install(app: &mut FeatureBuilder) -> anyhow::Result<Self>;
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

/// What a plugin is installed as.
pub(crate) struct Identity {
    pub(crate) id: &'static str,
    pub(crate) concrete: TypeId,
    pub(crate) settle: Settle,
}

impl Identity {
    pub(crate) fn plugin<P: Plugin>() -> Self {
        Self {
            id: P::ID,
            concrete: TypeId::of::<P>(),
            settle: settle::<P, P::Exports>,
        }
    }
}

pub(crate) trait ErasedPlugin: Send {
    fn id(&self) -> &'static str;
    fn build_boxed(self: Box<Self>, app: &mut PluginBuilder) -> anyhow::Result<()>;
}

impl<P: Plugin> ErasedPlugin for P {
    fn id(&self) -> &'static str {
        P::ID
    }

    fn build_boxed(self: Box<Self>, app: &mut PluginBuilder) -> anyhow::Result<()> {
        Plugin::build(*self, app)
    }
}
