use guinea_core::actor::shape::Declared;
use guinea_core::feature::Exported;

use super::builder::{FeatureBuilder, PluginBuilder};

/// Reusable, application-agnostic: the application knows the plugin, never the
/// other way round.
///
/// What it needs from another plugin it is handed in its own configuration,
/// and what it gives back is [`Provides`](Self::Provides), returned from
/// [`build`](Self::build) to whoever installed it:
///
/// ```ignore
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
///     let internal = app.plugin(StorePlugin::for_app(meta, "state"))?;
///     app.plugin(L10nPlugin::new("en").remembered_in(internal))?;
///     ..
/// }
/// ```
///
/// Nothing it returns is in the application's scope until the application
/// [`provide`](FeatureBuilder::provide)s it, so a plugin may be installed more
/// than once - two stores, say - and the application decides which one its
/// windows see.
pub trait Plugin: 'static {
    /// What diagnostics and devtools call it.
    const ID: &'static str;

    /// What every window may read of what it claims, as a feature's
    /// [`Exports`](crate::feature::Feature::Exports): `()`, or a tuple.
    type Exports: Exported;

    /// What it hands back to the application: `()`, a value, or a tuple.
    type Provides;

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<Self::Provides>;
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
