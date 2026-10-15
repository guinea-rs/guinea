use guinea_core::actor::shape::Declared;
use guinea_core::feature::Exported;

use super::builder::{FeatureBuilder, PluginBuilder};

/// Reusable, application-agnostic: the application knows the plugin, never the
/// other way round.
///
/// Written as a feature is: `plugin!` lists what it exports to every window,
/// the feeds it opens and what it provides to the application, and a
/// `#[builds]` function builds it from its parameters:
///
/// ```ignore
/// plugin! {
///     pub Settings {
///         provides { Store }
///     }
/// }
///
/// #[builds]
/// fn settings(app: &mut PluginBuilder, params: StoreParams) -> anyhow::Result<Settings> {
///     Ok(Settings(Store::open(params.path)?))
/// }
/// ```
///
/// What it needs from another plugin it is handed in its parameters, and the
/// application takes apart what it built:
///
/// ```ignore
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
///     let Settings(internal) = app.plugin::<Settings>(StoreParams::for_app(&meta, "state"))?;
///     app.plugin::<L10n<Strings>>(L10nParams::new("en").remembered_in(internal))?;
///     ..
/// }
/// ```
///
/// Nothing it provides is in the application's scope until the application
/// [`provide`](FeatureBuilder::provide)s it, so a plugin may be installed more
/// than once - two stores, say - and the application decides which one its
/// windows see.
pub trait Plugin: Sized + 'static {
    /// What it is built with, by value: its configuration, and what it is
    /// handed of other plugins. `()` for one built with nothing.
    type Params;

    /// What every window may read: what `plugin!` lists.
    type Exports: Exported;

    fn build(app: &mut PluginBuilder, params: Self::Params) -> anyhow::Result<Self>;
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
