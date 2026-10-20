use proc_macro::TokenStream;
use syn::{ItemFn, parse_macro_input};

mod actor_dsl;
mod elm;
mod builds;
mod feature_dsl;
mod given;
mod handler;
mod harness_test;
mod inspect;
mod installs;
mod mark;
mod paths;
mod reducer;
mod remote;
mod request;
mod route;
mod slot;

/// A feature's manifest: its name, the reducers it exports and the feeds it
/// opens.
///
/// ```
/// # use guinea_app::app::GuineaHost;
/// # use guinea_app::app::roots::Root;
/// # use guinea_app::feature::Install;
/// # use guinea_core::feature::Bound;
/// # use guinea_core::feed::Feed;
/// # use guinea_macros::{feature, installs};
/// # pub mod contracts {
/// #     #[derive(Clone, Default, PartialEq, Debug)]
/// #     pub struct Tabs(pub Vec<String>);
/// #
/// #     #[guinea_macros::reducer]
/// #     fn tabs(this: &mut Tabs, opened: String) {
/// #         this.0.push(opened);
/// #     }
/// #
/// #     pub struct OpenTab(pub String);
/// # }
/// feature! {
///     pub Tabs {
///         exports { contracts::Tabs }
///         feeds { contracts::OpenTab }
///     }
/// }
/// # #[installs]
/// # fn tabs(cx: &Install) -> anyhow::Result<Tabs> {
/// #     let tabs = cx.state::<contracts::Tabs>().plain();
/// #     let open_tab = cx.feed::<contracts::OpenTab>();
/// #     open_tab.set(contracts::OpenTab("inbox".into()));
/// #     Ok(Tabs(tabs, open_tab.feed()))
/// # }
/// #
/// # fn main() -> anyhow::Result<()> {
/// # let host = GuineaHost::new();
/// # let window = Root::under(&host.context());
/// # let Tabs(tabs, open_tab) = window.context(window.scope(), 0).install::<Tabs>(&())?;
/// # let _: (Bound<contracts::Tabs>, Feed<contracts::OpenTab>) = (tabs, open_tab);
/// # assert_eq!(open_tab.get().map(|open| open.0.clone()), Some("inbox".to_string()));
/// #
/// # tabs.push("inbox".into());
/// # let page = window.context(window.scope().child(), 1);
/// # assert_eq!(page.read::<contracts::Tabs>().map(|tabs| tabs.0.clone()), Some(vec!["inbox".to_string()]));
/// # Ok(())
/// # }
/// ```
///
/// It makes the feature's type: one `Bound` per export, then one `Feed` per
/// feed, then each dependency in `provides`, in the order listed. Provided
/// dependencies are published automatically after successful installation.
/// What installs it is an [`installs`] function.
#[proc_macro]
pub fn feature(input: TokenStream) -> TokenStream {
    feature_dsl::feature_impl(input)
}

/// A plugin's manifest: its name, the reducers it exports to every window,
/// the feeds it opens and what it provides to the application.
///
/// ```
/// # use guinea_app::app::PluginBuilder;
/// # use guinea_app::app::roots::Root;
/// # use guinea_macros::{builds, plugin, reducer};
/// # pub trait Localization: Clone + Default + 'static {}
/// #
/// # #[derive(Clone, Copy, Default, Debug, PartialEq)]
/// # pub enum Locale {
/// #     #[default]
/// #     En,
/// #     De,
/// # }
/// #
/// # impl Localization for Locale {}
/// #
/// # #[derive(Clone, Default)]
/// # pub struct Language<S: Localization>(pub S);
/// #
/// # #[reducer]
/// # fn language<S: Localization>(this: &mut Language<S>, to: S) {
/// #     this.0 = to;
/// # }
/// #
/// # #[derive(Clone, Debug, PartialEq)]
/// # pub struct Store(&'static str);
/// plugin! {
///     pub L10n<S: Localization> {
///         exports { Language<S> }
///     }
/// }
///
/// plugin! {
///     pub Settings {
///         provides { Store }
///     }
/// }
/// # #[builds]
/// # fn l10n<S: Localization>(app: &mut PluginBuilder, first: S) -> anyhow::Result<L10n<S>> {
/// #     Ok(L10n(app.state::<Language<S>>().seed(Language(first)).plain()))
/// # }
/// #
/// # #[builds]
/// # fn settings(_app: &mut PluginBuilder, file: &'static str) -> anyhow::Result<Settings> {
/// #     Ok(Settings(Store(file)))
/// # }
/// #
/// # fn main() -> anyhow::Result<()> {
/// # let (host, store) = guinea_app::app::start("doc", |app| {
/// #     app.plugin::<L10n<Locale>>(Locale::De)?;
/// #     let Settings(store) = app.plugin::<Settings>("settings.db")?;
/// #     Ok(store)
/// # })?;
/// # assert_eq!(store, Store("settings.db"));
/// #
/// # let window = Root::under(&host.context());
/// # let page = window.context(window.scope(), 0);
/// # assert_eq!(page.read::<Language<Locale>>().map(|language| language.0), Some(Locale::De));
/// # Ok(())
/// # }
/// ```
///
/// It makes the plugin's type - one `Bound` per export, one `Feed` per feed,
/// then each thing provided, in the order listed - which the application
/// may inspect: `let Settings(store) = app.plugin::<Settings>(params)?`.
/// Declared services are published automatically after a successful build.
/// What builds it is a [`builds`] function.
#[proc_macro]
pub fn plugin(input: TokenStream) -> TokenStream {
    feature_dsl::plugin_impl(input)
}

/// The function that builds a plugin: whatever it returns is the plugin, and
/// its second argument is what it is built with - nothing, when it takes the
/// builder alone.
///
/// ```
/// # use guinea_app::app::PluginBuilder;
/// # use guinea_app::app::roots::Root;
/// # use guinea_macros::{builds, plugin, reducer};
/// # pub trait Localization: Clone + Default + 'static {}
/// #
/// # #[derive(Clone, Copy, Default, Debug, PartialEq)]
/// # pub enum Locale {
/// #     #[default]
/// #     En,
/// #     De,
/// # }
/// #
/// # impl Localization for Locale {}
/// #
/// # #[derive(Clone, Default)]
/// # pub struct Language<S: Localization>(pub S);
/// #
/// # #[reducer]
/// # fn language<S: Localization>(this: &mut Language<S>, to: S) {
/// #     this.0 = to;
/// # }
/// #
/// # pub struct L10nParams<S>(Vec<S>);
/// #
/// # impl<S: Localization> L10nParams<S> {
/// #     fn first(&self) -> Language<S> {
/// #         Language(self.0[0].clone())
/// #     }
/// # }
/// #
/// # plugin! {
/// #     pub L10n<S: Localization> {
/// #         exports { Language<S> }
/// #     }
/// # }
/// #[builds]
/// fn l10n<S: Localization>(app: &mut PluginBuilder, params: L10nParams<S>) -> anyhow::Result<L10n<S>> {
///     let language = app.state::<Language<S>>().seed(params.first()).plain();
///     Ok(L10n(language))
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let (host, _) = guinea_app::app::start("doc", |app| {
/// #     app.plugin::<L10n<Locale>>(L10nParams(vec![Locale::De, Locale::En]))
/// # })?;
/// #
/// # let window = Root::under(&host.context());
/// # let page = window.context(window.scope(), 0);
/// # assert_eq!(page.read::<Language<Locale>>().map(|language| language.0), Some(Locale::De));
/// # Ok(())
/// # }
/// ```
///
/// The first argument after the builder is explicit configuration. Further
/// arguments are declared dependencies supplied by the framework. Use `()`
/// as configuration when only dependencies are needed.
#[proc_macro_attribute]
pub fn builds(_attr: TokenStream, item: TokenStream) -> TokenStream {
    builds::builds_impl(item)
}

/// The function that installs a feature: whatever it returns is the feature,
/// what its `Install` carries is what it is installed with, and every
/// argument after the context is a service the application provided.
///
/// ```
/// # use guinea_app::app::GuineaHost;
/// # use guinea_app::app::roots::Root;
/// # use guinea_app::feature::Install;
/// # use guinea_core::actor::In;
/// # use guinea_core::feature::Push;
/// # use guinea_macros::{actor, feature, handler, installs};
/// # pub mod contracts {
/// #     #[derive(Clone, Default, PartialEq, Debug)]
/// #     pub struct Tabs(pub Vec<String>);
/// #
/// #     #[guinea_macros::reducer]
/// #     fn tabs(this: &mut Tabs, opened: String) {
/// #         this.0.push(opened);
/// #     }
/// # }
/// #
/// # #[derive(Clone)]
/// # pub struct Store;
/// #
/// # #[derive(Clone)]
/// # pub struct Clock;
/// #
/// # pub struct Open;
/// #
/// # pub struct TabsActor {
/// #     first: String,
/// #     push: Push<contracts::Tabs>,
/// # }
/// #
/// # impl TabsActor {
/// #     fn new(first: &str, push: Push<contracts::Tabs>) -> Self {
/// #         Self { first: first.to_string(), push }
/// #     }
/// # }
/// #
/// # actor! {
/// #     TabsActor {
/// #         handlers { Open }
/// #     }
/// # }
/// #
/// # #[handler]
/// # fn open(In(this, Open, _): In<TabsActor, Open>) {
/// #     this.push.send(this.first.clone());
/// # }
/// #
/// # feature! {
/// #     pub Tabs {
/// #         exports { contracts::Tabs }
/// #     }
/// # }
/// #[installs]
/// fn tabs(cx: &Install<String>, store: Store, clock: Option<Clock>) -> anyhow::Result<Tabs> {
///     let (tabs, _) = cx.state::<contracts::Tabs>().driven_by(|push| TabsActor::new(cx.params(), push));
///     Ok(Tabs(tabs))
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let bare = GuineaHost::new();
/// # let window = Root::under(&bare.context());
/// # let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| window.context(window.scope(), 0).install::<Tabs>(&"inbox".to_string()))).err().unwrap().downcast::<String>().unwrap();
/// # assert!(!refused.is_empty());
/// #
/// # let (host, ()) = guinea_app::app::start("doc", |app| {
/// #     guinea_macros::plugin! { Resources { provides { Store } } }
/// #     #[guinea_macros::builds]
/// #     fn resources(_app: &mut guinea_app::app::PluginBuilder, value: Store) -> anyhow::Result<Resources> { Ok(Resources(value)) }
/// #     app.plugin::<Resources>(Store)?;
/// #     Ok(())
/// # })?;
/// # let window = Root::under(&host.context());
/// # let page = window.context(window.scope(), 0);
/// # let Tabs(tabs) = page.install::<Tabs>(&"inbox".to_string())?;
/// # tabs.emit(Open);
/// # assert_eq!(page.read::<contracts::Tabs>().map(|tabs| tabs.0.clone()), Some(vec!["inbox".to_string()]));
/// # Ok(())
/// # }
/// ```
///
/// A service is taken by clone, so it is taken by value; `Option<T>` is `None`
/// when no provider of `T` is declared. An unavailable declared dependency
/// panics before the function runs, with all missing inputs named at once.
///
/// A function taking `&mut FeatureBuilder` installs a feature of the
/// application, which every window reads from, instead:
///
/// ```
/// # use guinea_app::app::FeatureBuilder;
/// # use guinea_app::app::roots::Root;
/// # use guinea_core::actor::In;
/// # use guinea_core::feature::Push;
/// # use guinea_macros::{actor, feature, handler, installs, reducer};
/// # #[derive(Clone, Default)]
/// # pub struct ActivityState {
/// #     pub busy: bool,
/// # }
/// #
/// # #[reducer]
/// # fn activity_state(this: &mut ActivityState, busy: bool) {
/// #     this.busy = busy;
/// # }
/// #
/// # #[derive(Clone)]
/// # pub struct Store {
/// #     syncing: bool,
/// # }
/// #
/// # pub struct Refresh;
/// #
/// # pub struct ActivityActor {
/// #     store: Store,
/// #     push: Push<ActivityState>,
/// # }
/// #
/// # impl ActivityActor {
/// #     fn new(store: Store, push: Push<ActivityState>) -> Self {
/// #         Self { store, push }
/// #     }
/// # }
/// #
/// # actor! {
/// #     ActivityActor {
/// #         handlers { Refresh }
/// #     }
/// # }
/// #
/// # #[handler]
/// # fn refresh(In(this, Refresh, _): In<ActivityActor, Refresh>) {
/// #     this.push.send(this.store.syncing);
/// # }
/// #
/// # feature! {
/// #     pub Activity {
/// #         exports { ActivityState }
/// #     }
/// # }
/// #[installs]
/// fn activity(app: &mut FeatureBuilder, store: Store) -> anyhow::Result<Activity> {
///     let (activity, _) = app.state::<ActivityState>().driven_by(|push| ActivityActor::new(store, push));
///     Ok(Activity(activity))
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let (host, Activity(activity)) = guinea_app::app::start("doc", |app| {
/// #     guinea_macros::plugin! { Resources { provides { Store } } }
/// #     #[guinea_macros::builds]
/// #     fn resources(_app: &mut guinea_app::app::PluginBuilder, value: Store) -> anyhow::Result<Resources> { Ok(Resources(value)) }
/// #     app.plugin::<Resources>(Store { syncing: true })?;
/// #     app.install::<Activity>()
/// # })?;
/// # activity.emit(Refresh);
/// #
/// # let window = Root::under(&host.context());
/// # let page = window.context(window.scope(), 0);
/// # assert_eq!(page.read::<ActivityState>().map(|state| state.busy), Some(true));
/// # Ok(())
/// # }
/// ```
///
/// Named for what the function does rather than for the trait: `#[feature]`
/// is ambiguous with Rust's own `feature` attribute.
#[proc_macro_attribute]
pub fn installs(_attr: TokenStream, item: TokenStream) -> TokenStream {
    installs::installs_impl(item)
}

/// Makes a function a reducer: `impl Reducer` written from its signature. The
/// state is what the first argument borrows mutably, the update is the second
/// argument's type:
///
/// <!-- shown: a reducer -->
/// ```rust,ignore
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Count(pub u32);
///
/// #[reducer]
/// fn count(this: &mut Count, by: u32) {
///     this.0 += by;
/// }
/// ```
/// <!-- /shown -->
///
/// An update with more than one shape is an enum, and the function matches on
/// it - destructured right in the argument when there is one shape only:
///
/// <!-- shown: a reducer of an enum -->
/// ```rust,ignore
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Table {
///     pub rows: Vec<String>,
///     pub descending: bool,
/// }
///
/// #[derive(Clone, Debug)]
/// pub enum Changed {
///     Rows(Vec<String>),
///     Sorted { descending: bool },
/// }
///
/// #[reducer]
/// fn table(this: &mut Table, changed: Changed) {
///     match changed {
///         Changed::Rows(rows) => this.rows = rows,
///         Changed::Sorted { descending } => this.descending = descending,
///     }
/// }
/// ```
/// <!-- /shown -->
///
/// Two arguments, no more. A reducer knows its state and what changed it, not
/// who asked, so there is no context to take; it is not `async`, and returns
/// nothing. `impl Reducer` written by hand is the same thing.
#[proc_macro_attribute]
pub fn reducer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    reducer::reducer_impl(input).into()
}

/// A test run once per seed, each time on a fresh `Harness`, with the order of
/// everything it sets off decided by the seed.
///
/// A failing seed is named in the panic; `SEED=<n> cargo test` runs that order
/// alone, and again.
///
/// `exclusive = "key"` runs it one at a time with every other test in the
/// process that names the same key - for what a process has one of, a global
/// store say, which tests on their own threads would otherwise fight over.
///
/// ```ignore
/// #[guinea::test(iterations = 200)]
/// fn the_latest_query_wins(h: &mut Harness) {
///     h.install::<Search>(&()).unwrap();
///     h.dispatch::<Results>().emit(Query("gu".into()));
///     h.dispatch::<Results>().emit(Query("guinea".into()));
///     h.settled();
///     assert_eq!(h.state::<Results>().query, "guinea");
/// }
/// ```
#[proc_macro_attribute]
pub fn test(attr: TokenStream, item: TokenStream) -> TokenStream {
    harness_test::test_impl(attr, item)
}

/// Declares a slot: a place in a layout's view that a segment below fills.
///
/// ```
/// # use guinea_macros::slot;
/// # use guinea_router::slot::Slot;
/// #[slot]
/// pub struct Toolbar;
/// # fn placed<S: Slot>() -> &'static str {
/// #     std::any::type_name::<S>()
/// # }
/// #
/// # fn main() {
/// # assert!(placed::<Toolbar>().ends_with("Toolbar"));
/// # }
/// ```
#[proc_macro_attribute]
pub fn slot(_attr: TokenStream, item: TokenStream) -> TokenStream {
    slot::slot_impl(item)
}

/// Writes down what an `impl Page` for the iced backend left out, and the
/// hidden `Mountable` that `cx.mount::<Services>(&params)` takes.
///
/// ```ignore
/// #[page]
/// impl Page for Services {
///     type Message = Msg;
///
///     fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) { .. }
///     fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Msg> { .. }
/// }
/// ```
///
/// An omitted `Params` becomes `()` and an omitted `Message` becomes
/// `Infallible`; a node with the defaulted message also gets the empty
/// `update` that goes with it. Nothing else - a macro that derived a
/// declaration from a body would be a second source of truth wearing the
/// clothes of one.
#[proc_macro_attribute]
pub fn iced_page(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Page, "guinea-iced", "iced", elm::Model::Elm)
}

/// [`iced_page`] for a layout.
#[proc_macro_attribute]
pub fn iced_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-iced", "iced", elm::Model::Elm)
}

/// Writes down what an `impl Page` for the egui backend left out, and the
/// hidden `Mountable` that `cx.mount::<Services>(&params)` takes.
///
/// ```ignore
/// #[page]
/// impl Page for Services {
///     fn render(&mut self, cx: &mut PageCx<'_, Self>) { .. }
/// }
/// ```
///
/// An omitted `Params` becomes `()`, and an omitted `Installs` becomes `()`
/// with the `install` that starts from `Default`.
#[proc_macro_attribute]
pub fn eframe_page(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Page, "guinea-eframe", "eframe", elm::Model::Immediate)
}

/// [`eframe_page`] for a layout.
#[proc_macro_attribute]
pub fn eframe_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-eframe", "eframe", elm::Model::Immediate)
}

/// [`eframe_page`] for the terminal backend.
#[proc_macro_attribute]
pub fn ratatui_page(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Page, "guinea-ratatui", "ratatui", elm::Model::Immediate)
}

/// [`ratatui_page`] for a layout.
#[proc_macro_attribute]
pub fn ratatui_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-ratatui", "ratatui", elm::Model::Immediate)
}

/// [`eframe_page`] for Slint, where a page has no drawing of its own.
#[proc_macro_attribute]
pub fn slint_page(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Page, "guinea-slint", "slint", elm::Model::Immediate)
}

/// [`slint_page`] for a layout.
#[proc_macro_attribute]
pub fn slint_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-slint", "slint", elm::Model::Immediate)
}

/// [`iced_page`] for the windows-reactor backend.
///
/// The same macro because the two backends are the same kind of thing: the
/// reactor's second preview is Elm - state in structs, events as enums - so a
/// page there has the same five items a page here does, and leaving out the
/// empty ones is the same job.
#[proc_macro_attribute]
pub fn winui_page(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Page, "guinea-winui", "winui", elm::Model::Elm)
}

/// [`winui_page`] for a layout.
#[proc_macro_attribute]
pub fn winui_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-winui", "winui", elm::Model::Elm)
}

/// Declares an actor's manifest:
///
/// ```
/// # use std::cell::RefCell;
/// # use std::rc::Rc;
/// # use guinea_app::app::GuineaHost;
/// # use guinea_app::app::roots::Root;
/// # use guinea_core::actor::In;
/// # use guinea_macros::{Event, actor, handler};
/// # pub trait ProcessesPort {
/// #     fn kill(&self, pid: u32);
/// #     fn refresh(&self);
/// # }
/// #
/// # #[derive(Clone, Default)]
/// # pub struct Recorded(Rc<RefCell<Vec<String>>>);
/// #
/// # impl ProcessesPort for Recorded {
/// #     fn kill(&self, pid: u32) {
/// #         self.0.borrow_mut().push(format!("kill {pid}"));
/// #     }
/// #
/// #     fn refresh(&self) {
/// #         self.0.borrow_mut().push("refresh".into());
/// #     }
/// # }
/// #
/// # pub struct Kill(pub u32);
/// # pub struct Killed(pub u32);
/// # pub struct Refresh;
/// #
/// # #[derive(Clone, Event)]
/// # pub struct ProcessKilled(pub u32);
/// #
/// # #[derive(Clone, Event)]
/// # pub struct SettingsChanged;
/// #
/// # pub struct ProcessActor<P> {
/// #     port: P,
/// # }
/// actor! {
///     ProcessActor<P: ProcessesPort + 'static> {
///         handlers   { Kill => { send Killed }, Killed, Refresh }
///         publishes  { ProcessKilled }
///         subscribes { SettingsChanged }
///     }
/// }
/// # #[handler]
/// # fn kill<P: ProcessesPort + 'static>(In(this, Kill(pid), cx): In<ProcessActor<P>, Kill>) {
/// #     this.port.kill(pid);
/// #     cx.send(Killed(pid));
/// # }
/// #
/// # #[handler]
/// # fn killed<P: ProcessesPort + 'static>(In(_, Killed(pid), cx): In<ProcessActor<P>, Killed>) {
/// #     cx.publish(ProcessKilled(pid));
/// # }
/// #
/// # #[handler]
/// # fn refresh<P: ProcessesPort + 'static>(In(this, Refresh, _): In<ProcessActor<P>, Refresh>) {
/// #     this.port.refresh();
/// # }
/// #
/// # #[handler]
/// # fn settings_changed<P: ProcessesPort + 'static>(In(this, _, _): In<ProcessActor<P>, SettingsChanged>) {
/// #     this.port.refresh();
/// # }
/// #
/// # fn main() {
/// # let host = GuineaHost::new();
/// # let window = Root::under(&host.context());
/// # let port = Recorded::default();
/// # let processes = window.context(window.scope(), 0).spawn(ProcessActor { port: port.clone() });
/// #
/// # let heard = Rc::new(RefCell::new(Vec::new()));
/// # let into = heard.clone();
/// # let _hearing = window.scope().bus().subscribe_fn(move |ProcessKilled(pid)| into.borrow_mut().push(pid));
/// #
/// # processes.send(Kill(4));
/// # assert_eq!(*heard.borrow(), [4]);
/// #
/// # window.scope().bus().publish(SettingsChanged);
/// # processes.send(Refresh);
/// # assert_eq!(*port.0.borrow(), ["kill 4", "refresh", "refresh"]);
/// # }
/// ```
///
/// What it subscribes to it handles, edges and all - `subscribes { Saved =>
/// { send Flush } }` - so a message is named once: in `handlers` or in
/// `subscribes`, never both. Each section appears once.
#[proc_macro]
pub fn actor(input: TokenStream) -> TokenStream {
    actor_dsl::actor_impl(input)
}

/// Puts a type on the bus: `impl Event for T {}`.
#[proc_macro_derive(Event)]
pub fn event(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    let gc = paths::core();

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote::quote! {
        impl #impl_generics #gc::actor::event_bus::Event for #name #ty_generics #where_clause {}
    }
    .into()
}

/// Makes a type a request on the bus, answered with `reply`: the
/// request names its answer where the request is declared.
///
/// <!-- shown: a request and its reply -->
/// ```rust,ignore
/// #[derive(Clone, Debug, guinea::Request)]
/// #[request(reply = Outcome)]
/// pub struct Kill(pub u32);
///
/// #[derive(Clone, Debug, PartialEq)]
/// pub enum Outcome {
///     Done,
///     Denied,
/// }
/// ```
/// <!-- /shown -->
///
/// Exactly one subscriber answers it: a handler that returns the reply. The
/// generated code sends what it returns back to whoever asked, once - and the
/// `async` form does the same after its body resolves. The actor's manifest
/// subscribes it to the request on the bus, so it answers while the segment
/// that installed it stands:
///
/// <!-- shown: the one that answers -->
/// ```rust,ignore
/// #[derive(Debug, Default)]
/// pub struct Processes {
///     protected: Vec<u32>,
/// }
///
/// actor! {
///     Processes {
///         subscribes { RpcRequest<Kill> }
///     }
/// }
///
/// // Returning the reply is what makes it the answer: the generated code
/// // sends it back, once.
/// #[handler]
/// fn kill(In(this, Kill(pid), _): In<Processes, Kill>) -> Outcome {
///     match this.protected.contains(&pid) {
///         true => Outcome::Denied,
///         false => Outcome::Done,
///     }
/// }
///
/// feature! {
///     pub Killing {}
/// }
///
/// // It answers for as long as the segment that installed it stands.
/// #[installs]
/// fn killing(cx: &Install) -> anyhow::Result<Killing> {
///     cx.spawn(Processes { protected: vec![4] });
///     Ok(Killing)
/// }
/// ```
/// <!-- /shown -->
///
/// Asking is `AsyncContext::request`, awaited off the UI thread. It resolves to
/// the reply, or to an error saying why there is none: nothing answers the
/// request, the one that does sleeps in a `keep` segment, or the timeout ran
/// out. The first two are known before anything is published, so they come
/// back at once:
///
/// <!-- shown: asking -->
/// ```rust,ignore
/// pub struct Ask(pub u32);
/// pub struct Told(pub Result<Outcome, Why>);
///
/// #[derive(Debug)]
/// pub struct Asker {
///     push: Push<Said>,
/// }
///
/// actor! {
///     Asker {
///         handlers { Ask, Told }
///     }
/// }
///
/// // Asked off the UI thread, and answered with what the one answerer
/// // returned - or with why there was nothing to wait for.
/// #[handler]
/// async fn ask(cx: AsyncContext<Asker>, Ask(pid): Ask) {
///     let said = cx.request(Kill(pid), Duration::from_secs(5)).await;
///     cx.send(Told(said.map_err(|unanswered| unanswered.why)));
/// }
///
/// #[handler]
/// fn told(In(this, Told(said), _): In<Asker, Told>) {
///     this.push.send(said);
/// }
/// ```
/// <!-- /shown -->
///
/// Anything else subscribed to the request only hears it. A handler that
/// returns nothing is told what was asked and cannot answer - its reply goes
/// nowhere:
///
/// <!-- shown: one that only hears it -->
/// ```rust,ignore
/// #[derive(Debug, Default)]
/// pub struct Audit {
///     pub seen: Vec<u32>,
/// }
///
/// actor! {
///     Audit {
///         subscribes { RpcRequest<Kill> }
///     }
/// }
///
/// // Returns nothing, so it hears the request and cannot answer it: there
/// // is one answerer, and it is not this.
/// #[handler]
/// fn heard(In(this, request, _): In<Audit, RpcRequest<Kill>>) {
///     this.seen.push(request.payload.0);
/// }
///
/// feature! {
///     pub Auditing {}
/// }
///
/// #[installs]
/// fn auditing(cx: &Install) -> anyhow::Result<Auditing> {
///     cx.spawn(Audit::default());
///     Ok(Auditing)
/// }
/// ```
/// <!-- /shown -->
///
/// A second answerer on the same bus is a setup bug, and is refused - with a
/// panic naming both - when it subscribes, rather than racing the first one
/// for every request. A closure answers with `EventBus::answer_fn`.
#[proc_macro_derive(Request, attributes(request))]
pub fn request(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    request::derive_request(input).into()
}

/// Makes an enum a place a window can be: a variant per place, and a
/// one-field tuple variant - `Processes(ProcessesRoute)` - for each layout
/// that has places of its own below it.
///
/// `#[at("/process/:pid")]` gives a variant an address: a `:name` is the
/// named field of that name, one escaped path segment of a
/// [`LinkValue`](../guinea_router/link/trait.LinkValue.html) type. Every
/// linked place, nested levels included, is in `Routable::deep_links()`.
/// A branch's link is the prefix of the enum it holds, and `"/"` is the place
/// a level is at when nothing is added. At any one position a literal is
/// tried before a capture, whatever the order of declaration.
#[proc_macro_derive(Routable, attributes(at))]
pub fn routable(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    route::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Lets devtools open the type a node at a time: a struct by its fields, an
/// enum by the variant it is, as its `Debug` would write them. A field marked
/// `#[inspect(skip)]` is left out - a handle, a client, a closure.
///
/// An actor or a reducer's state that derives it is shown through it; one
/// that does not is shown through its `Debug`.
#[proc_macro_derive(Inspect, attributes(inspect))]
pub fn inspect(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    inspect::derive_inspect(input).into()
}

/// Makes an enum of unit variants the application's marks: each variant is a
/// name, written as the variant is.
#[proc_macro_derive(Mark)]
pub fn mark(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    mark::derive_mark(input).into()
}

/// Lets a tool send this type to the running application as JSON: as an
/// action to whichever scope answers it, as an event on the bus, or
/// both. The type derives `serde::Deserialize` too.
///
/// ```
/// # use std::cell::Cell;
/// # use std::rc::Rc;
/// # use guinea_app::app::roots::Root;
/// # use guinea_macros as guinea;
/// # use serde::Deserialize;
/// #[derive(Clone, Debug, Deserialize, guinea::Remote)]
/// #[remote(action)]
/// pub struct Kill(pub u32);
/// # fn main() -> anyhow::Result<()> {
/// # let (host, observed) = guinea_app::app::start("doc", |app| Ok(app.observed()))?;
/// # assert!(observed.actions().contains(&"Kill"));
/// #
/// # let window = Root::under(&host.context());
/// # let killed = Rc::new(Cell::new(None));
/// # let into = killed.clone();
/// # let _page = window.install(|cx| {
/// #     let into = into.clone();
/// #     cx.answers(move |Kill(pid)| into.set(Some(pid)));
/// #     Ok(())
/// # })?;
/// #
/// # observed.act(None, "Kill", "4").unwrap();
/// # assert_eq!(killed.get(), Some(4));
/// # Ok(())
/// # }
/// ```
#[proc_macro_derive(Remote, attributes(remote))]
pub fn remote(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    remote::derive_remote(input).into()
}

/// Makes a function an actor's handler for one message: `impl Handler<M>`
/// written from its signature.
///
/// A handler takes one argument, what it is handed: `In(this, msg, cx)`, the
/// actor, the message and its `Cx`, typed once as `In<Actor, Msg>`. It is
/// taken apart right there, so the handler binds what it uses and leaves the
/// rest as `_` - the message destructured in place when that reads better:
///
/// <!-- shown: a handler that takes the message -->
/// ```rust,ignore
/// pub struct Add(pub u32);
///
/// #[derive(Debug)]
/// pub struct Counting {
///     push: Push<Count>,
/// }
///
/// actor! {
///     Counting {
///         handlers { Add }
///     }
/// }
///
/// #[handler]
/// fn add(In(this, Add(by), _): In<Counting, Add>) {
///     this.push.send(by);
/// }
/// ```
/// <!-- /shown -->
///
/// A handler that sends on, publishes, or starts background work binds the
/// `Cx`. It is typed by the message it handles, which is what `actor!`'s flow
/// checks - a handler of `Refresh` declared `Refresh => { task Counted }` may
/// spawn work that answers `Counted`, and nothing else compiles:
///
/// <!-- shown: a handler that starts work -->
/// ```rust,ignore
/// pub struct Refresh;
/// pub struct Counted(pub u32);
///
/// #[derive(Debug)]
/// pub struct Counting {
///     push: Push<Count>,
/// }
///
/// actor! {
///     Counting {
///         handlers { Refresh => { task Counted }, Counted }
///     }
/// }
///
/// // `cx` is `Cx<Counting, Refresh>`: the `In` it came in names both.
/// #[handler]
/// fn refresh(In(_, _, cx): In<Counting, Refresh>) {
///     cx.task().spawn(async {
///         guinea::executor::random_delay().await;
///         Counted(7)
///     });
/// }
///
/// #[handler]
/// fn counted(In(this, Counted(n), _): In<Counting, Counted>) {
///     this.push.send(n);
/// }
/// ```
/// <!-- /shown -->
///
/// An `async fn` runs off the UI thread. It takes the actor's
/// `AsyncContext` instead of the actor - the actor itself stays on the UI
/// thread - and ends with the actor: dropped at its next await once the actor
/// is gone, unless it listens for that itself. Work that needs the actor's
/// state is a plain handler that reads it and hands the rest to `cx.task()`,
/// as above: the state is read when the message comes, on the UI thread.
///
/// <!-- shown: an async handler -->
/// ```rust,ignore
/// pub struct Load(pub u32);
/// pub struct Loaded(pub u32);
///
/// #[derive(Debug)]
/// pub struct Loader {
///     push: Push<Count>,
/// }
///
/// actor! {
///     Loader {
///         handlers { Load, Loaded }
///     }
/// }
///
/// // Runs off the UI thread, and ends with the actor: `cx` knows when it is
/// // gone, and sends back to it while it is not.
/// #[handler]
/// async fn load(cx: AsyncContext<Loader>, Load(n): Load) {
///     guinea::executor::random_delay().await;
///     cx.send(Loaded(n * 2));
/// }
///
/// #[handler]
/// fn loaded(In(this, Loaded(n), _): In<Loader, Loaded>) {
///     this.push.send(n);
/// }
/// ```
/// <!-- /shown -->
///
/// A return type makes the handler an answer to a request: the
/// value it returns is the reply, sent exactly once, by the generated code and
/// nothing else. That holds for both the plain and the `async` form. A plain
/// handler whose answer needs background work returns `Reply<T>`: it reads
/// what it needs from the actor and answers with `Reply::later(..)`, or with
/// `Reply::now(..)` when there is nothing to wait for.
#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    handler::generate_standalone_handler(input)
}
