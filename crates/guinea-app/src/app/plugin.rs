use guinea_core::actor::shape::Declared;
use guinea_core::__private::Exported;

use super::builder::{FeatureBuilder, PluginBuilder};

/// Reusable, application-agnostic: the application knows the plugin, never the
/// other way round.
///
/// Written as a feature is: `plugin!` lists what it exports to every window,
/// the feeds it opens and what it provides to the application, and a
/// `#[builds]` function builds it from its parameters:
///
/// ```
/// use guinea_app::app::PluginBuilder;
/// use guinea_macros::{builds, plugin};
///
/// #[derive(Clone, Debug, PartialEq)]
/// pub struct Store(&'static str);
///
/// plugin! {
///     pub Settings {
///         provides { Store }
///     }
/// }
///
/// #[builds]
/// fn settings(_app: &mut PluginBuilder, file: &'static str) -> anyhow::Result<Settings> {
///     Ok(Settings(Store(file)))
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let (_runtime, Settings(store)) =
/// #     guinea_app::app::start("doc", |app| app.plugin::<Settings>("settings.db"))?;
/// # assert_eq!(store, Store("settings.db"));
/// # Ok(())
/// # }
/// ```
///
/// What it needs from another plugin it is handed in its parameters, and the
/// application takes apart what it built. Nothing it provides is in the
/// application's scope until the application
/// [`provide`](FeatureBuilder::provide)s it, so a plugin may be installed more
/// than once - two stores, say - and the application decides which one its
/// windows see:
///
/// ```
/// # use guinea_app::app::PluginBuilder;
/// # use guinea_macros::{builds, plugin};
/// use guinea_app::app::FeatureBuilder;
/// use guinea_app::feature::FeatureHost;
/// # #[derive(Clone, Debug, PartialEq)]
/// # pub struct Store(&'static str);
/// # plugin! { pub Settings { provides { Store } } }
/// # #[builds]
/// # fn settings(_app: &mut PluginBuilder, file: &'static str) -> anyhow::Result<Settings> {
/// #     Ok(Settings(Store(file)))
/// # }
///
/// plugin! {
///     pub Remembering {}
/// }
///
/// #[builds]
/// fn remembering(_app: &mut PluginBuilder, _store: Store) -> anyhow::Result<Remembering> {
///     Ok(Remembering)
/// }
///
/// fn app(app: &mut FeatureBuilder) -> anyhow::Result<()> {
///     let Settings(user) = app.plugin::<Settings>("settings.db")?;
///     let Settings(internal) = app.plugin::<Settings>("state.db")?;
///     app.plugin::<Remembering>(internal)?;
///     app.provide(user);
///     Ok(())
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let (runtime, ()) = guinea_app::app::start("doc", app)?;
///
/// let window = FeatureHost::under(&runtime.context());
/// assert_eq!(window.scope().service::<Store>(), Some(Store("settings.db")));
/// # Ok(())
/// # }
/// ```
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
/// ```
/// use guinea_app::app::FeatureBuilder;
/// use guinea_app::feature::FeatureHost;
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
/// # let (runtime, _) = guinea_app::app::start("doc", |app| app.install::<Activity>())?;
///
/// let window = FeatureHost::under(&runtime.context());
/// let _page = window.install(|cx| {
///     assert_eq!(cx.read::<ActivityState>().map(|state| state.busy), Some(true));
///     Ok(())
/// })?;
/// # Ok(())
/// # }
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
