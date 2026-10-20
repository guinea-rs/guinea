use guinea_core::actor::shape::Declared;
use guinea_core::__private::Exported;

use super::builder::{FeatureBuilder, PluginBuilder};

/// A reusable component whose declared outputs become available after a
/// successful build. The first argument after the builder is configuration;
/// subsequent arguments are declared dependencies, passed by clone.
///
/// ```
/// use guinea_app::app::{PluginBuilder, start};
/// use guinea_macros::{builds, plugin};
///
/// #[derive(Clone)]
/// struct Store(&'static str);
///
/// plugin! { Settings { provides { Store } } }
///
/// #[builds]
/// fn settings(_app: &mut PluginBuilder, file: &'static str) -> anyhow::Result<Settings> {
///     Ok(Settings(Store(file)))
/// }
///
/// plugin! { Remembering {} }
///
/// #[builds]
/// fn remembering(_app: &mut PluginBuilder, (): (), store: Store) -> anyhow::Result<Remembering> {
///     assert_eq!(store.0, "settings.db");
///     Ok(Remembering)
/// }
///
/// # fn main() -> anyhow::Result<()> {
/// let (_host, ()) = start("doc", |app| {
///     app.plugin::<Settings>("settings.db")?;
///     app.plugin::<Remembering>(())?;
///     Ok(())
/// })?;
/// # Ok(())
/// # }
/// ```
///
/// Providing a type twice is a composition bug and panics before the second
/// builder runs. Distinct dependencies need distinct wrapper types.
/// Resource failures remain ordinary installation errors.
pub trait Plugin: Sized + 'static {
    #[doc(hidden)]
    fn declaration() -> Option<crate::feature::declarations::Declaration> { None }
    /// What it is built with, by value: its configuration, and what it is
    /// handed of other plugins. `()` for one built with nothing.
    type Params;

    /// What every window may read: what `plugin!` lists.
    type Exports: Exported;

    #[doc(hidden)]
    fn publish_dependencies(&self, _cx: &crate::feature::ScopeContext) {}

    fn build(app: &mut PluginBuilder, params: Self::Params) -> anyhow::Result<Self>;
}

/// This application's own wiring: a feature installed in the application's
/// scope, which every window sits under.
///
/// Written as a feature in a window is, by `feature!` and `#[installs]`, with
/// the builder as the context:
///
/// ```
/// use guinea_app::app::FeatureBuilder;
/// use guinea_app::app::roots::Root;
/// use guinea_macros::{Inspect, feature, installs, reducer};
///
/// #[derive(Clone, Default, Inspect)]
/// pub struct ActivityState {
///     pub busy: bool,
/// }
///
/// #[reducer]
/// fn activity_state(this: &mut ActivityState, busy: bool) {
///     this.busy = busy;
/// }
///
/// feature! {
///     pub Activity {
///         exports { ActivityState }
///     }
/// }
///
/// #[installs]
/// fn activity(app: &mut FeatureBuilder) -> anyhow::Result<Activity> {
///     let state = app.state::<ActivityState>().seed(ActivityState { busy: true }).plain();
///     Ok(Activity(state))
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let (host, _) = guinea_app::app::start("doc", |app| app.install::<Activity>())?;
///
/// let window = Root::under(&host.context());
/// let _page = window.install(|cx| {
///     assert_eq!(cx.read::<ActivityState>().map(|state| state.busy), Some(true));
///     Ok(())
/// })?;
/// # Ok(())
/// # }
/// ```
pub trait AppFeature: Sized + 'static {
    #[doc(hidden)]
    fn declaration() -> Option<crate::feature::declarations::Declaration> { None }
    /// What every window may read: what `feature!` lists.
    type Exports: Exported;

    /// Where the feature was written. `#[installs]` fills it in.
    const DECLARED: Option<Declared> = None;

    #[doc(hidden)]
    fn publish_dependencies(&self, _cx: &crate::feature::ScopeContext) {}

    fn install(app: &mut FeatureBuilder) -> anyhow::Result<Self>;
}

/// Returned from [`Plugin::build`] or [`AppFeature::install`] to end the
/// application before it starts: what is already installed is torn down, no
/// window opens, and `run` returns `Ok`.
///
/// ```
/// use guinea_app::app::{PluginBuilder, Stop};
/// use guinea_macros::{builds, plugin};
/// # fn another_copy_is_running() -> bool { true }
///
/// plugin! {
///     pub SingleInstance {}
/// }
///
/// #[builds]
/// fn single_instance(_app: &mut PluginBuilder) -> anyhow::Result<SingleInstance> {
///     if another_copy_is_running() {
///         return Err(Stop.into());
///     }
///     Ok(SingleInstance)
/// }
/// # fn main() {
/// # let started = guinea_app::app::start("doc", |app| app.plugin::<SingleInstance>(()));
/// # assert!(started.is_err_and(|error| error.is::<Stop>()));
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop;

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the application stopped before it started")
    }
}

impl std::error::Error for Stop {}
