use proc_macro::TokenStream;
use syn::{ItemFn, parse_macro_input};

mod actor_dsl;
mod elm;
mod feature_dsl;
mod handler;
mod harness_test;
mod installs;
mod mark;
mod reducer;
mod remote;
mod request;
mod routes_dsl;
mod segment;

/// A feature's manifest: its name, and the reducers it exports.
///
/// ```ignore
/// feature! {
///     pub Tabs {
///         exports { contracts::Tabs }
///     }
/// }
/// ```
///
/// It makes the feature's type - one `Bound` per export, in the order listed -
/// and what it exports. What installs it is an [`installs`] function.
#[proc_macro]
pub fn feature(input: TokenStream) -> TokenStream {
    feature_dsl::feature_impl(input)
}

/// The function that installs a feature: whatever it returns is the feature,
/// its second argument is what it is installed with.
///
/// ```ignore
/// #[installs]
/// fn tabs(cx: &FeatureInitContext, context: &str) -> anyhow::Result<Tabs> {
///     let (tabs, _) = cx.state::<contracts::Tabs>().driven_by(|push| TabsActor::new(push));
///     Ok(Tabs(tabs))
/// }
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

/// Writes `type Installs = ();` and the `install` that goes with it, for a
/// page or layout that installs nothing.
///
/// Only that. Declaring what a segment installs is what makes the declaration
/// an obligation of the body; declaring that it installs *nothing* is
/// ceremony, and stable Rust has no conditional default body to remove it.
///
/// ```ignore
/// #[segment]
/// impl Page for Splash {
///     type Params = ();
///     fn view(cx: &mut PageCx<'_>) { .. }
/// }
/// ```
#[proc_macro_attribute]
pub fn segment(_attr: TokenStream, item: TokenStream) -> TokenStream {
    segment::segment_impl(item)
}

/// Writes down what an `impl Page` for the iced backend left out.
///
/// ```ignore
/// #[page]
/// impl Page for Services {
///     type Params = ServicesParams;
///
///     fn install(ctx: &FeatureInitContext, _: &Self::Params) -> anyhow::Result<()> { .. }
///     fn view(&self, cx: &PageCx<'_>) -> View<Self::Message> { .. }
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
    elm::node_impl(item, elm::Kind::Page, "guinea-iced", "iced")
}

/// [`iced_page`] for a layout, which has no route parameters of its own.
#[proc_macro_attribute]
pub fn iced_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-iced", "iced")
}

/// [`iced_page`] for the windows-reactor backend.
///
/// The same macro because the two backends are the same kind of thing: the
/// reactor's second preview is Elm - state in structs, events as enums - so a
/// page there has the same five items a page here does, and leaving out the
/// empty ones is the same job.
#[proc_macro_attribute]
pub fn winui_page(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Page, "guinea-winui", "winui")
}

/// [`winui_page`] for a layout.
#[proc_macro_attribute]
pub fn winui_layout(_attr: TokenStream, item: TokenStream) -> TokenStream {
    elm::node_impl(item, elm::Kind::Layout, "guinea-winui", "winui")
}

/// Declares an actor's manifest:
///
/// ```ignore
/// actor! {
///     ProcessActor<P: ProcessesPort + 'static> {
///         handlers   { Kill, Refresh }
///         publishes  { ProcessKilled }
///         subscribes { SettingsChanged }
///     }
/// }
/// ```
#[proc_macro]
pub fn actor(input: TokenStream) -> TokenStream {
    actor_dsl::actor_impl(input)
}

/// `routes! { Route { layout(TabsLayout) { page(Processes) link("/:context/processes")
/// { context: String } ... } } }` - the tree's `{}` nesting *is* the segment
/// chain (no attribute stack to track); `page(...)`'s type also names the
/// generated variant, so there's one name per leaf, not two kept in sync by
/// hand. Generates the enum itself, `link` and `deep_links` for the routes
/// that agreed to have an address, and `RouteChain` (enum -> segment chain).
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    routes_dsl::routes_impl(input)
}

/// Puts a type on the global bus: `impl Event for T {}`.
#[proc_macro_derive(Event)]
pub fn event(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    let gc = handler::guinea_core_crate_path();

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote::quote! {
        impl #impl_generics #gc::actor::event_bus::Event for #name #ty_generics #where_clause {}
    }
    .into()
}

/// Makes a type a request on the global bus, answered with `reply`: the
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
/// `async` form does the same after its body resolves. The feature that owns
/// the actor subscribes it to the request on the global bus, so it answers
/// while the segment that installed it stands:
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
///         handlers { RpcRequest<Kill> }
///     }
/// }
///
/// // Returning the reply is what makes it the answer: the generated code
/// // sends it back, once.
/// #[handler]
/// fn kill(this: &mut Processes, Kill(pid): Kill) -> Outcome {
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
/// fn killing(cx: &FeatureInitContext) -> anyhow::Result<Killing> {
///     let processes = cx.spawn_actor(Processes { protected: vec![4] });
///     processes.subscribe_on::<RpcRequest<Kill>>(Bus::Global);
///     Ok(Killing)
/// }
/// ```
/// <!-- /shown -->
///
/// Asking is `AsyncBus::request`, awaited off the UI thread. It resolves to
/// the reply, or to an error saying why there is none: nothing answers the
/// request, the one that does sleeps in a `keep` segment, or the timeout ran
/// out. The first two are known before anything is published, so they come
/// back at once:
///
/// <!-- shown: asking -->
/// ```rust,ignore
/// pub struct Ask(pub u32);
/// pub struct Told(pub String);
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
///     let said = match AsyncBus::request(Kill(pid), Duration::from_secs(5)).await {
///         Ok(outcome) => format!("{outcome:?}"),
///         Err(error) => error.to_string(),
///     };
///     cx.send(Told(said));
/// }
///
/// #[handler]
/// fn told(this: &mut Asker, Told(said): Told) {
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
///         handlers { RpcRequest<Kill> }
///     }
/// }
///
/// // Returns nothing, so it hears the request and cannot answer it: there
/// // is one answerer, and it is not this.
/// #[handler]
/// fn heard(this: &mut Audit, request: RpcRequest<Kill>) {
///     this.seen.push(request.payload.0);
/// }
///
/// feature! {
///     pub Auditing {}
/// }
///
/// #[installs]
/// fn auditing(cx: &FeatureInitContext) -> anyhow::Result<Auditing> {
///     let audit = cx.spawn_actor(Audit::default());
///     audit.subscribe_on::<RpcRequest<Kill>>(Bus::Global);
///     Ok(Auditing)
/// }
/// ```
/// <!-- /shown -->
///
/// A second answerer on the same bus is a setup bug, and is refused - with a
/// panic naming both - when it subscribes, rather than racing the first one
/// for every request. A closure answers with `GlobalEventBus::answer_fn`.
#[proc_macro_derive(Request, attributes(request))]
pub fn request(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    request::derive_request(input).into()
}

/// Makes an enum of unit variants the application's marks: each variant is a
/// name, written as the variant is.
#[proc_macro_derive(Mark)]
pub fn mark(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    mark::derive_mark(input).into()
}

/// Lets a tool send this type to the running application as JSON: as an
/// action to whichever scope answers it, as an event on the global bus, or
/// both. The type derives `serde::Deserialize` too.
///
/// ```ignore
/// #[derive(Clone, Debug, Deserialize, guinea::Remote)]
/// #[remote(action)]
/// pub struct Kill(pub u32);
/// ```
#[proc_macro_derive(Remote, attributes(remote))]
pub fn remote(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    remote::derive_remote(input).into()
}

/// Makes a function an actor's handler for one message: `impl Handler<M>`
/// written from its signature.
///
/// A handler takes what it uses and nothing else. The actor, then the message
/// itself - destructured right in the argument when that reads better:
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
/// fn add(this: &mut Counting, Add(by): Add) {
///     this.push.send(by);
/// }
/// ```
/// <!-- /shown -->
///
/// A handler that sends on, publishes, or starts background work takes a
/// third argument, its `Cx`. Written bare: the actor and the message are the
/// two arguments before it, and the macro writes them in. `Cx` is typed by the
/// message it handles, which is what `actor!`'s flow checks - a handler of
/// `Refresh` declared `Refresh => { bg Counted }` may spawn work that answers
/// `Counted`, and nothing else compiles:
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
///         handlers { Refresh => { bg Counted }, Counted }
///     }
/// }
///
/// // `cx` is `Cx<Counting, Refresh>`: the macro writes in what the two
/// // arguments before it already say.
/// #[handler]
/// fn refresh(_this: &mut Counting, _: Refresh, cx: Cx) {
///     cx.spawn_bg::<Counted, _>(async {
///         guinea::core::executor::random_delay().await;
///         Counted(7)
///     });
/// }
///
/// #[handler]
/// fn counted(this: &mut Counting, Counted(n): Counted) {
///     this.push.send(n);
/// }
/// ```
/// <!-- /shown -->
///
/// An `async fn` runs off the UI thread. It takes the actor's
/// `AsyncContext` instead of the actor - the actor itself stays on the UI
/// thread - and ends with the actor: dropped at its next await once the actor
/// is gone, unless it listens for that itself.
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
///     guinea::core::executor::random_delay().await;
///     cx.send(Loaded(n * 2));
/// }
///
/// #[handler]
/// fn loaded(this: &mut Loader, Loaded(n): Loaded) {
///     this.push.send(n);
/// }
/// ```
/// <!-- /shown -->
///
/// A return type makes the handler an answer to `AsyncBus::request`: the
/// value it returns is the reply, sent exactly once, by the generated code and
/// nothing else. That holds for both the plain and the `async` form.
#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    handler::generate_standalone_handler(input)
}
