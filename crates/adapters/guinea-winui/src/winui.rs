//! The windows-reactor backend: what a segment is, how it is mounted, and what
//! it reads state through.
//!
//! Rewritten for the component model, and rewritten the same way the iced
//! adapter is written - because they are now the same kind of thing. The PR
//! that replaced the render-and-hook API says so in its own words: *state lives
//! in structs, events are enums*. That is Elm, and guinea already had an Elm
//! backend.
//!
//! So a page here looks like a page there. The struct that implements [`Page`]
//! **is** the page's state; it has a `Message` of its own, one `update` that is
//! the only place it changes, and a `view` that borrows it. What differs is
//! only what a view returns and how an event reaches the node - owned `View`
//! and a `Callback` here, a borrowed `Element` and a mapped message there.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_core::guard::Verdict;
use guinea_core::scope::Reducer;
use guinea_core::trace::Cause;

use guinea_app::feature::{FeatureInitContext, Reads, ScopeContext};
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, RouteSink, Router, SegmentEntry, SegmentProps, Ui,
    single_entry_chain,
};
use guinea_router::slot::Slot;
use windows_reactor::{
    Border, Callback, Component, ComponentContext, ContentDialog, ContentDialogExt,
    ContentDialogResult, Grid, TextBlock, View, ViewContext, provide,
};

/// windows-reactor as a [`Ui`].
pub struct WinUi;

impl Ui for WinUi {
    /// Reactor's own name, and owned rather than borrowed: the reconciler owns
    /// the tree, and a view it is handed owns everything it shows.
    type View<'a> = View;
    type Nodes = ();
}

/// A leaf of the route tree, and an Elm node.
///
/// The struct that implements this is the page's state. Nothing above it ever
/// names its `Message`, so adding a page costs no edit anywhere else.
///
/// Each part has one job:
///
/// - [`install`](Page::install) sets up what the page needs while it is
///   mounted - the features listed in [`Installs`](Page::Installs). They live
///   in the page's scope and go when the page is left.
/// - [`init`](Page::init) builds the node, when `Default` is not the start.
/// - [`update`](Page::update) is the only place the page's own state changes,
///   one [`Message`](Page::Message) at a time.
/// - [`view`](Page::view) draws that state. It reads what features publish
///   with [`read`](PageCx::read), turns a widget's event into a
///   message with [`on`](PageCx::on), and asks a feature for something with
///   `emit` on the dispatch it was handed.
///
/// `#[page]` writes what a page leaves out: `Params = ()` for a page that
/// captures nothing, `Installs = ()` with its empty `install`, and a
/// `Message` nobody can send with its empty `update`. The smallest page is a
/// view:
///
/// <!-- shown: a page that is only a view -->
/// ```rust,ignore
/// use guinea_winui::{Page, PageCx, page};
/// use windows_reactor::{TextBlock, View};
///
/// #[derive(Default)]
/// struct About;
///
/// #[page]
/// impl Page for About {
///     fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
///         TextBlock::new().text("guinea").into()
///     }
/// }
/// ```
/// <!-- /shown -->
///
/// A whole one. The feature keeps a count and adds to it when asked; the page
/// shows the count, keeps a step of its own, and asks for the step to be
/// added:
///
/// <!-- shown: a whole page -->
/// ```rust,ignore
/// use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
/// use windows_reactor::{Button, StackPanel, TextBlock, View};
///
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Count(pub u32);
///
/// #[reducer]
/// fn count(this: &mut Count, by: u32) {
///     this.0 += by;
/// }
///
/// /// What the page asks the feature for.
/// pub struct Add(pub u32);
///
/// feature! {
///     pub Counter {
///         exports { Count }
///     }
/// }
///
/// #[installs]
/// fn counter(cx: &FeatureInitContext) -> anyhow::Result<Counter> {
///     let count = cx.state::<Count>().plain();
///
///     let adding = count.clone();
///     cx.answers(move |Add(by): Add| adding.push(by));
///
///     Ok(Counter(count))
/// }
///
/// pub struct CounterPage {
///     step: u32,
/// }
///
/// impl Default for CounterPage {
///     fn default() -> Self {
///         Self { step: 1 }
///     }
/// }
///
/// pub enum Msg {
///     Bigger,
/// }
///
/// #[page]
/// impl Page for CounterPage {
///     type Installs = Counter;
///     type Message = Msg;
///
///     fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Counter> {
///         ctx.install(&())
///     }
///
///     fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
///         match message {
///             Msg::Bigger => self.step += 1,
///         }
///     }
///
///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
///         let (count, dispatch) = cx.read::<Count>();
///         let step = self.step;
///
///         StackPanel::new()
///             .children((
///                 TextBlock::new().text(format!("{} by {step}", count.0)),
///                 Button::new()
///                     .on_click(move || dispatch.emit(Add(step)))
///                     .content(TextBlock::new().text("Add")),
///                 Button::new()
///                     .on_click(cx.on(|()| Msg::Bigger))
///                     .content(TextBlock::new().text("Bigger")),
///             ))
///             .into()
///     }
/// }
///
/// // Under test it mounts with no window, in a harness that runs what it
/// // sets off in a seeded order - `harness::Mounted`, behind the `harness`
/// // feature. `#[guinea::test]` is this `check` as an attribute.
/// #[test]
/// fn adds_the_step_it_was_told() {
///     guinea_app::app::check(4, |h| {
///         let mut page =
///             guinea_winui::harness::Mounted::<CounterPage>::mount(h.segment(), ()).unwrap();
///
///         page.click_text("Bigger").settle();
///         page.click_text("Add").settle();
///         page.settle();
///
///         assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
///     });
/// }
/// ```
/// <!-- /shown -->
pub trait Page: Default + Sized + 'static {
    /// When `true`, the router keeps this page's reducer states in memory
    /// while the page is not mounted. The page's scope (and therefore its
    /// actors) is still torn down, but when the user returns the UI will see
    /// the last cached state immediately instead of starting from defaults.
    ///
    /// Only when it comes back with the same [`Params`](Self::Params): a
    /// different capture is a different page's state.
    ///
    /// <!-- shown: a page whose state outlives it -->
    /// ```rust,ignore
    /// use guinea_winui::{Page, PageCx, page};
    /// use windows_reactor::{TextBlock, View};
    ///
    /// #[derive(Default)]
    /// struct Processes;
    ///
    /// #[page]
    /// impl Page for Processes {
    ///     // Back from another tab, the list is there at once rather than
    ///     // empty until the next refresh arrives.
    ///     const CACHE_STATE_IN_MEMORY: bool = true;
    ///
    ///     fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         TextBlock::new().text("processes").into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    const CACHE_STATE_IN_MEMORY: bool = false;

    /// Where `impl Page` was written, for devtools to link to. `#[page]`
    /// fills it in; an impl without it loses only the source link.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What this page captured from the route, named by `routes!`. `()` for a
    /// page that captures nothing, which `#[page]` writes when it is left out.
    ///
    /// `PartialEq` because the router's one question about a capture is
    /// whether it is still the same one - which decides what reinstalls and
    /// which cached state may come back.
    ///
    /// A page sees its capture twice: in [`install`](Self::install), for what
    /// the features need, and in [`init`](Self::init), for what the node
    /// keeps. See `init` for an example.
    type Params: PartialEq + 'static;

    /// What this segment installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking.
    ///
    /// What is returned is owned by the segment's scope, which is what gives a
    /// feature its own lifetime.
    ///
    /// One feature, a reducer claimed directly as `Bound<R>`, or a flat tuple
    /// of up to twelve of either. It is also what the page may read: what it
    /// installed itself, and what a segment above listed in `Exports`. See
    /// [`install`](Self::install) for an example.
    type Installs: guinea_app::feature::Lists;

    /// This page's own events. An enum, and nobody else's business.
    ///
    /// Made from a widget's event by [`PageCx::on`], and handled in
    /// [`update`](Self::update). A page with nothing to handle leaves it out,
    /// and `#[page]` writes one that cannot be sent along with the empty
    /// `update`.
    type Message: 'static;

    /// Installs what the page needs while it is mounted, and returns it as
    /// [`Installs`](Self::Installs).
    ///
    /// Runs on every mount, before [`init`](Self::init), in the page's own
    /// scope: what it installs goes when the page is left. Everything an
    /// effect needs exactly once per mount belongs here - a first `Refresh`
    /// emitted to a feature, say.
    ///
    /// <!-- shown: a page that installs -->
    /// ```rust,ignore
    /// use guinea_winui::{FeatureInitContext, Page, PageCx, page};
    /// use windows_reactor::{TextBlock, View};
    ///
    /// #[derive(Default, Clone, PartialEq, Debug)]
    /// pub struct Listing(pub String);
    ///
    /// #[reducer]
    /// fn listing(this: &mut Listing, to: String) {
    ///     this.0 = to;
    /// }
    ///
    /// /// Which row is selected: the page's own, with no feature around it.
    /// #[derive(Default, Clone, PartialEq, Debug)]
    /// pub struct Selection(pub Option<u32>);
    ///
    /// #[reducer]
    /// fn selection(this: &mut Selection, to: Option<u32>) {
    ///     this.0 = to;
    /// }
    ///
    /// feature! {
    ///     pub Processes {
    ///         exports { Listing }
    ///     }
    /// }
    ///
    /// /// A feature with parameters: which machine to list.
    /// #[installs]
    /// fn processes(cx: &FeatureInitContext, context: &str) -> anyhow::Result<Processes> {
    ///     let listing = cx.state::<Listing>().plain();
    ///     listing.push(format!("processes on {context}"));
    ///     Ok(Processes(listing))
    /// }
    ///
    /// #[derive(PartialEq)]
    /// pub struct ProcessesParams {
    ///     pub context: String,
    /// }
    ///
    /// #[derive(Default)]
    /// pub struct ProcessesPage;
    ///
    /// #[page]
    /// impl Page for ProcessesPage {
    ///     type Params = ProcessesParams;
    ///     type Installs = (Processes, Bound<Selection>);
    ///
    ///     fn install(
    ///         ctx: &FeatureInitContext,
    ///         params: &ProcessesParams,
    ///     ) -> anyhow::Result<Self::Installs> {
    ///         let processes = ctx.install::<Processes>(&params.context)?;
    ///         let selection = ctx.state::<Selection>().plain();
    ///         Ok((processes, selection))
    ///     }
    ///
    ///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         let (listing, _) = cx.read::<Listing>();
    ///         TextBlock::new().text(listing.0.clone()).into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// The node it starts as, when `Default` is not it.
    ///
    /// A constructor, not an effect: it runs on every install, and anything
    /// that must happen exactly once per mount belongs in
    /// [`install`](Self::install).
    ///
    /// Where a page keeps what it was reached with, since `view` is not
    /// handed the capture:
    ///
    /// <!-- shown: a page that keeps its capture -->
    /// ```rust,ignore
    /// use guinea_winui::{FeatureInitContext, Page, PageCx, page};
    /// use windows_reactor::{TextBlock, View};
    ///
    /// #[derive(PartialEq)]
    /// pub struct ProcessParams {
    ///     pub pid: u32,
    /// }
    ///
    /// #[derive(Default)]
    /// pub struct ProcessPage {
    ///     pid: u32,
    /// }
    ///
    /// #[page]
    /// impl Page for ProcessPage {
    ///     type Params = ProcessParams;
    ///
    ///     fn init(_ctx: &FeatureInitContext, params: &ProcessParams) -> Self {
    ///         Self { pid: params.pid }
    ///     }
    ///
    ///     fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         TextBlock::new()
    ///             .text(format!("process {}", self.pid))
    ///             .into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Asked before this page is left, and answered from its own state.
    ///
    /// The asymmetry with entering is the whole reason it is a method here: on
    /// the way out the node exists, so it can say whether it minds - which is
    /// what "unsaved changes" is. On the way in there is nothing to ask.
    ///
    /// [`Verdict::Allow`] lets the navigation through, [`Verdict::Block`]
    /// stays, and [`Verdict::ask`] puts a question to the user and waits for
    /// the answer:
    ///
    /// <!-- shown: a page that minds being left -->
    /// ```rust,ignore
    /// use guinea_winui::{Ask, Page, PageCx, Verdict, page};
    /// use windows_reactor::{TextBlock, View};
    ///
    /// #[derive(Default)]
    /// pub struct Editor {
    ///     text: String,
    ///     saved: String,
    /// }
    ///
    /// #[page]
    /// impl Page for Editor {
    ///     fn leaving(&self) -> Verdict {
    ///         if self.text == self.saved {
    ///             Verdict::Allow
    ///         } else {
    ///             Verdict::ask(Ask::new("Discard the changes?", "Discard", "Stay"))
    ///         }
    ///     }
    ///
    ///     fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         TextBlock::new().text(self.text.clone()).into()
    ///     }
    /// }
    ///
    /// #[test]
    /// fn asks_only_with_changes() {
    ///     let mut editor = Editor::default();
    ///     assert!(matches!(editor.leaving(), Verdict::Allow));
    ///
    ///     editor.text = "draft".into();
    ///     assert!(matches!(editor.leaving(), Verdict::Ask(..)));
    /// }
    /// ```
    /// <!-- /shown -->
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    /// The only place the node changes.
    ///
    /// Effects are actions emitted to features - `cx.read::<R>().1.emit(..)` -
    /// rather than values returned from here: an effect that crosses a segment
    /// boundary is a domain's job, and one that does not is a state change.
    ///
    /// [`UpdateCx::read`] reads what the page may read, as the view does, but
    /// without subscribing: `update` is a moment, and the view that follows
    /// reads again.
    ///
    /// <!-- shown: a page that asks a feature -->
    /// ```rust,ignore
    /// use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
    /// use windows_reactor::{TextBlock, View};
    ///
    /// #[derive(Default, Clone, PartialEq, Debug)]
    /// pub struct Results(pub String);
    ///
    /// #[reducer]
    /// fn results(this: &mut Results, to: String) {
    ///     this.0 = to;
    /// }
    ///
    /// pub struct Search(pub String);
    ///
    /// feature! {
    ///     pub Searching {
    ///         exports { Results }
    ///     }
    /// }
    ///
    /// #[installs]
    /// fn searching(cx: &FeatureInitContext) -> anyhow::Result<Searching> {
    ///     let results = cx.state::<Results>().plain();
    ///
    ///     let answering = results.clone();
    ///     cx.answers(move |Search(query): Search| answering.push(format!("found {query}")));
    ///
    ///     Ok(Searching(results))
    /// }
    ///
    /// #[derive(Default)]
    /// pub struct SearchPage {
    ///     query: String,
    /// }
    ///
    /// pub enum Msg {
    ///     Typed(String),
    ///     Submitted,
    /// }
    ///
    /// #[page]
    /// impl Page for SearchPage {
    ///     type Installs = Searching;
    ///     type Message = Msg;
    ///
    ///     fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Searching> {
    ///         ctx.install(&())
    ///     }
    ///
    ///     fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) {
    ///         match message {
    ///             Msg::Typed(text) => self.query = text,
    ///             Msg::Submitted => {
    ///                 let (_, search) = cx.read::<Results>();
    ///                 search.emit(Search(self.query.clone()));
    ///             }
    ///         }
    ///     }
    ///
    ///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         let (results, _) = cx.read::<Results>();
    ///         TextBlock::new().text(results.0.clone()).into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    /// Draws the page from its own state and from what it may read.
    ///
    /// Called again whenever the node changes and whenever a reducer it read
    /// with [`read`](PageCx::read) does, so it holds nothing: a
    /// value it needs later is either the node's or a reducer's.
    ///
    /// Three ways out of a view, and each is a callback handed to a widget:
    /// [`on`](PageCx::on) makes the widget's event one of the page's own
    /// messages, `dispatch.emit(..)` asks a feature for something, and
    /// [`navigate`](PageCx::navigate) goes elsewhere.
    ///
    /// <!-- shown: a page that answers a widget -->
    /// ```rust,ignore
    /// use std::rc::Rc;
    ///
    /// use guinea_winui::{Page, PageCx, UpdateCx, page};
    /// use windows_reactor::{StackPanel, TextBlock, TextBox, View};
    ///
    /// #[derive(Default)]
    /// pub struct Greeting {
    ///     name: Rc<str>,
    /// }
    ///
    /// pub enum Msg {
    ///     Named(Rc<str>),
    /// }
    ///
    /// #[page]
    /// impl Page for Greeting {
    ///     type Message = Msg;
    ///
    ///     fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
    ///         match message {
    ///             Msg::Named(name) => self.name = name,
    ///         }
    ///     }
    ///
    ///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         StackPanel::new()
    ///             .children((
    ///                 TextBox::new(&self.name).on_text_changed(cx.on(Msg::Named)),
    ///                 TextBlock::new().text(format!("hello, {}", self.name)),
    ///             ))
    ///             .into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View;
}

/// A branch: an Elm node that also decides where its child goes.
///
/// Everything a [`Page`] is, plus two things only a branch has:
///
/// - [`outlet`](LayoutCx::outlet) is the segment below, for the view to place
///   wherever it wants - beside a sidebar, under a tab strip.
/// - What it installs outlives the pages under it. A feature a layout
///   installs is there for every page below and stays while they come and go,
///   and whatever it lists in `Exports` those pages may read.
///
/// `Params` is not the layout's to choose: `routes!` hands it what every page
/// below was reached with. `#[layout]` writes the rest a layout leaves out,
/// the same as `#[page]`.
///
/// A shell with a sidebar it can close, and a tab it marks while its page is
/// the one showing:
///
/// <!-- shown: a shell -->
/// ```rust,ignore
/// use guinea_winui::{
///     FeatureInitContext, Layout, LayoutCx, Page, PageCx, UpdateCx, layout, page,
/// };
/// use windows_reactor::{Border, Button, StackPanel, TextBlock, View};
///
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Sidebar {
///     pub open: bool,
/// }
///
/// #[reducer]
/// fn sidebar(this: &mut Sidebar, open: bool) {
///     this.open = open;
/// }
///
/// pub struct SetOpen(pub bool);
///
/// feature! {
///     pub Chrome {
///         exports { Sidebar }
///     }
/// }
///
/// #[installs]
/// fn chrome(cx: &FeatureInitContext) -> anyhow::Result<Chrome> {
///     let sidebar = cx.state::<Sidebar>().seed(Sidebar { open: true }).plain();
///
///     let setting = sidebar.clone();
///     cx.answers(move |SetOpen(open): SetOpen| setting.push(open));
///
///     Ok(Chrome(sidebar))
/// }
///
/// /// State the shell keeps itself, with no feature around it.
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Title(pub String);
///
/// #[reducer]
/// fn title(this: &mut Title, title: String) {
///     this.0 = title;
/// }
///
/// #[derive(Default)]
/// pub struct Home;
///
/// #[page]
/// impl Page for Home {
///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
///         // Installed by the shell above, and readable here because `Chrome`
///         // exports it. A page outside the shell asking for it panics on
///         // its first render, naming the chain it walked.
///         let (sidebar, _) = cx.read::<Sidebar>();
///         let width = if sidebar.open { "narrow" } else { "wide" };
///
///         TextBlock::new().text(format!("home, {width}")).into()
///     }
/// }
///
/// #[derive(Default)]
/// pub struct Shell;
///
/// pub enum ShellMsg {
///     Toggle,
/// }
///
/// #[layout]
/// impl Layout for Shell {
///     type Params = ();
///     /// A flat list: a feature, and a reducer claimed directly. Pages below
///     /// may read `Sidebar`, which `Chrome` exports, and `Title`.
///     type Installs = (Chrome, Bound<Title>);
///     type Message = ShellMsg;
///
///     fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self::Installs> {
///         let title = ctx.state::<Title>().seed(Title("guinea".into())).plain();
///         Ok((ctx.install(&())?, title))
///     }
///
///     fn update(&mut self, message: ShellMsg, cx: &mut UpdateCx<'_, Self>) {
///         match message {
///             ShellMsg::Toggle => {
///                 let (sidebar, dispatch) = cx.read::<Sidebar>();
///                 dispatch.emit(SetOpen(!sidebar.open));
///             }
///         }
///     }
///
///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
///         let (sidebar, _) = cx.read::<Sidebar>();
///         let (title, _) = cx.read::<Title>();
///
///         let tab = if cx.child_is::<Home>() {
///             "> Home"
///         } else {
///             "Home"
///         };
///         let side = match sidebar.open {
///             true => Border::new().content(TextBlock::new().text(tab)),
///             false => Border::new(),
///         };
///
///         StackPanel::new()
///             .children((
///                 TextBlock::new().text(title.0.clone()),
///                 Button::new()
///                     .on_click(cx.on(|()| ShellMsg::Toggle))
///                     .content(TextBlock::new().text("Menu")),
///                 side,
///                 cx.outlet(),
///             ))
///             .into()
///     }
/// }
/// ```
/// <!-- /shown -->
///
/// Nothing reaches up past what is above it. A page that no layout above
/// installed `Sidebar` for panics on its first render, naming the chain it
/// walked and who claimed `Sidebar` without exporting it.
pub trait Layout: Default + Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What every page under this layout carries, derived by `routes!` as the
    /// intersection of their parameters. A layout declares nothing; it is
    /// handed what all of its children were reached with.
    ///
    /// So it is named, not written: `type Params = crate::routes::ShellParams;`
    /// for whatever `routes!` called it, and `()` when the pages below share
    /// nothing. Unlike a page's, `#[layout]` does not default it - a layout
    /// that forgot to name it would silently stop receiving what its pages
    /// share.
    ///
    /// Moving between two pages that carry the same value keeps the layout,
    /// and everything it installed, where it is; a different value installs
    /// it again. That is what makes a feature per `context` right to install
    /// here:
    ///
    /// <!-- shown: a layout per context -->
    /// ```rust,ignore
    /// use guinea_winui::{FeatureInitContext, Layout, LayoutCx, layout};
    /// use windows_reactor::View;
    ///
    /// /// What `routes!` writes when every page below captures `context`.
    /// #[derive(PartialEq)]
    /// pub struct MachineParams {
    ///     pub context: String,
    /// }
    ///
    /// #[derive(Default)]
    /// pub struct MachineLayout;
    ///
    /// #[layout]
    /// impl Layout for MachineLayout {
    ///     type Params = MachineParams;
    ///     type Installs = Connection;
    ///
    ///     fn install(ctx: &FeatureInitContext, params: &MachineParams) -> anyhow::Result<Connection> {
    ///         ctx.install::<Connection>(&params.context)
    ///     }
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         cx.outlet()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    type Params: PartialEq + 'static;

    /// What this segment installs. See [`Page::Installs`].
    ///
    /// For a layout it is also what the pages below share: they come and go,
    /// and what is installed here stays. Whatever its features list in
    /// `Exports`, and every reducer claimed directly as `Bound<R>`, those
    /// pages may read.
    type Installs: guinea_app::feature::Lists;

    /// This layout's own events. See [`Page::Message`].
    type Message: 'static;

    /// Installs what the layout, and every page under it, needs while it is
    /// mounted. See [`Page::install`].
    ///
    /// <!-- shown: a layout that installs -->
    /// ```rust,ignore
    /// use guinea_winui::{FeatureInitContext, Layout, LayoutCx, layout};
    /// use windows_reactor::View;
    ///
    /// /// How often metrics refresh, when the application does not say.
    /// #[derive(Clone, Default)]
    /// pub struct Refresh {
    ///     pub every_ms: u64,
    /// }
    ///
    /// #[derive(Default)]
    /// pub struct Shell;
    ///
    /// #[layout]
    /// impl Layout for Shell {
    ///     type Params = ();
    ///     type Installs = (SidebarFeature, MetricsFeature, Bound<Title>);
    ///
    ///     fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self::Installs> {
    ///         // A setting the application may `provide`, and a default when
    ///         // it does not.
    ///         let refresh = ctx.require_or_default::<Refresh>();
    ///         let title = ctx
    ///             .state::<Title>()
    ///             .seed(Title(format!("every {} ms", refresh.every_ms)));
    ///
    ///         Ok((ctx.install(&())?, ctx.install(&())?, title.plain()))
    ///     }
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         cx.outlet()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// The node it starts as, when `Default` is not it. See [`Page::init`].
    ///
    /// Runs when the layout is installed, which is not every navigation: a
    /// move between two of its pages keeps the node, and what the layout
    /// holds survives it.
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Asked before this layout is left. See [`Page::leaving`].
    ///
    /// Only when the layout itself goes - left, or installed again because
    /// what its pages share changed. Moving between two of its pages that
    /// share the same value asks the page, not the layout; when both would go,
    /// the page speaks first. A wizard that minds being abandoned halfway,
    /// whichever step it is on:
    ///
    /// <!-- shown: a layout that minds being left -->
    /// ```rust,ignore
    /// use guinea_winui::{Ask, Layout, LayoutCx, Verdict, layout};
    /// use windows_reactor::View;
    ///
    /// #[derive(Default)]
    /// pub struct Wizard {
    ///     step: u32,
    ///     finished: bool,
    /// }
    ///
    /// #[layout]
    /// impl Layout for Wizard {
    ///     type Params = ();
    ///
    ///     fn leaving(&self) -> Verdict {
    ///         if self.step == 0 || self.finished {
    ///             Verdict::Allow
    ///         } else {
    ///             Verdict::ask(Ask::new("Abandon the setup?", "Abandon", "Continue"))
    ///         }
    ///     }
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         cx.outlet()
    ///     }
    /// }
    ///
    /// #[test]
    /// fn asks_only_halfway() {
    ///     let mut wizard = Wizard::default();
    ///     assert!(matches!(wizard.leaving(), Verdict::Allow));
    ///
    ///     wizard.step = 2;
    ///     assert!(matches!(wizard.leaving(), Verdict::Ask(..)));
    /// }
    /// ```
    /// <!-- /shown -->
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    /// The only place the node changes. See [`Page::update`].
    ///
    /// A layout reads what it installed itself, and what a layout above it
    /// exports - not what the page below it installed.
    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    /// Draws the layout around [`outlet`](LayoutCx::outlet), the segment
    /// below. See [`Page::view`].
    ///
    /// The outlet is a value like any other view: placed once, beside or
    /// under whatever the layout draws itself, and left out for a layout that
    /// hides its content. [`child_is`](LayoutCx::child_is) says which page it
    /// is, for a tab strip that marks the current tab without keeping a copy
    /// of the route:
    ///
    /// <!-- shown: a tab strip -->
    /// ```rust,ignore
    /// use guinea_winui::{Layout, LayoutCx, Page, PageCx, layout, page};
    /// use windows_reactor::{StackPanel, TextBlock, View};
    ///
    /// #[derive(Default)]
    /// pub struct Tabs;
    ///
    /// #[layout]
    /// impl Layout for Tabs {
    ///     type Params = ();
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         let tab = |name: &str, current: bool| {
    ///             TextBlock::new().text(if current {
    ///                 format!("[{name}]")
    ///             } else {
    ///                 name.to_string()
    ///             })
    ///         };
    ///
    ///         StackPanel::new()
    ///             .children((
    ///                 tab("Processes", cx.child_is::<Processes>()),
    ///                 tab("Services", cx.child_is::<Services>()),
    ///                 cx.outlet(),
    ///             ))
    ///             .into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View;
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<WinUi> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        guinea_router::router::same_params::<P::Params>,
        <P::Installs as guinea_app::feature::Lists>::list,
        &const { MountPage::<P>(PhantomData) },
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<WinUi> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        guinea_router::router::same_params::<L::Params>,
        <L::Installs as guinea_app::feature::Lists>::list,
        &const { MountLayout::<L>(PhantomData) },
        false,
    )
    .written(L::DECLARED)
}

thread_local! {
    /// Nodes built by `install`, by the scope they were installed into.
    ///
    /// `init` needs the feature context, which exists while the router is
    /// installing; a component is created later, when the reconciler reaches
    /// it, and is handed no such thing. So the node is built where the context
    /// is and found where it is needed.
    ///
    /// Kept for as long as the scope is, not taken once: a component the
    /// reconciler drops and makes again in the same scope - a part a fill
    /// covered and uncovered - is the same segment, and gets the same node.
    static STAGED: RefCell<HashMap<usize, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Forgets a scope's node when the scope goes.
struct Unstage(usize);

impl Drop for Unstage {
    fn drop(&mut self) {
        let _gone = STAGED.try_with(|staged| staged.borrow_mut().remove(&self.0));
    }
}

fn stage<N: 'static>(ctx: &FeatureInitContext, node: Shared<N>) {
    let key = ctx.scope.key();
    STAGED.with(|staged| {
        staged.borrow_mut().insert(key, Box::new(node));
    });
    ctx.scope.own(guinea_core::scope::DropGuard(Unstage(key)));
}

/// The node installed into the scope at `props`, or a default one - which is
/// what a page mounted outside a route tree gets.
fn staged<N: Default + 'static>(props: &SegmentProps<WinUi>) -> Shared<N> {
    let key = props.scopes[props.cursor].key();
    STAGED
        .with(|staged| {
            staged
                .borrow()
                .get(&key)
                .and_then(|node| node.downcast_ref::<Shared<N>>())
                .cloned()
        })
        .unwrap_or_default()
}

pub(crate) fn install_page<P: Page>(ctx: &FeatureInitContext, params: &dyn Any) -> anyhow::Result<()> {
    let params = guinea_router::router::narrow::<P::Params, P>(params)?;
    own(ctx, P::install(ctx, params)?);
    stage_node(ctx, P::init(ctx, params), P::leaving);
    Ok(())
}

/// A node as its component and its leave guard share it.
type Shared<N> = Rc<RefCell<N>>;

/// Parks the node for its component, and registers the guard that asks the
/// node whether it may go.
///
/// The router asks the scope, and a guard registered there reaches the node
/// only through what it captured - so the node is shared, and asked afresh
/// each time, with a new question when it has one.
fn stage_node<N: 'static>(ctx: &FeatureInitContext, node: N, leaving: fn(&N) -> Verdict) {
    let node: Shared<N> = Rc::new(RefCell::new(node));

    let asked = Rc::downgrade(&node);
    ctx.on_leave(move || {
        let Some(node) = asked.upgrade() else {
            return Verdict::Allow;
        };
        match node.try_borrow() {
            Ok(node) => leaving(&node),
            Err(_) => {
                tracing::warn!(
                    node = std::any::type_name::<N>(),
                    "asked whether it may go while it was changing; let it go"
                );
                Verdict::Allow
            }
        }
    });

    stage(ctx, node);
}

/// Hands what a segment installed to its scope - a feature's lifetime is the
/// segment's, and dropping this here would end it at the end of `install`.
fn own<T: 'static>(ctx: &FeatureInitContext, installed: T) {
    ctx.scope.own(guinea_core::scope::DropGuard(installed));
}

pub(crate) fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &dyn Any) -> anyhow::Result<()> {
    let params = guinea_router::router::narrow::<L::Params, L>(params)?;
    own(ctx, L::install(ctx, params)?);
    stage_node(ctx, L::init(ctx, params), L::leaving);
    Ok(())
}

/// A zero-sized marker per segment type: what a `const` entry points at to get
/// its `&'static dyn Mount`.
pub struct MountPage<P>(pub PhantomData<P>);
pub struct MountLayout<L>(pub PhantomData<L>);

impl<P: Page> Mount<WinUi> for MountPage<P> {
    fn view<'a>(&self, props: SegmentProps<WinUi>, _nodes: &'a ()) -> View {
        View::component::<PageNode<P>>(props)
    }
}

impl<L: Layout> Mount<WinUi> for MountLayout<L> {
    fn view<'a>(&self, props: SegmentProps<WinUi>, _nodes: &'a ()) -> View {
        View::component::<LayoutNode<L>>(props)
    }
}

/// A one-segment chain for a page mounted without a route tree.
pub fn page_chain<P: Page>() -> &'static [SegmentEntry<WinUi>] {
    single_entry_chain(segment_entry::<P>())
}

/// What reaches a segment's component.
///
/// The node's own message plus two the adapter needs. A parent never sees any
/// of it: `Signal` is the segment's private alphabet, and `Signal::Node` is
/// where the page's own enum lives.
pub enum Signal<M> {
    /// State this segment reads has changed; publish again.
    ///
    /// It carries no state, because the new state is read from the reducer
    /// rather than delivered. The message says *when*, and what caused it, so
    /// the redraw is traced to that.
    Refresh(Option<Cause>),
    /// Open this as a window of its own. See [`crate::window`].
    ///
    /// A message rather than a call because the reactor accepts `open_window`
    /// only during `create`, `changed` or `update`, never during `view`.
    OpenWindow(View),
    /// One of the node's own.
    Node(M),
}

/// A component whose message alphabet contains [`Signal::Refresh`].
///
/// Every segment of a guinea route tree is one. It exists so that something
/// outside this crate - a plugin that redraws a view when the language changes,
/// say - can ask for a refresh without naming `PageNode<P>`.
pub trait Refreshable: Component {
    fn refresh() -> Self::Message;
}

impl<P: Page> Refreshable for PageNode<P> {
    fn refresh() -> Self::Message {
        Signal::Refresh(guinea_core::trace::current())
    }
}

impl<L: Layout> Refreshable for LayoutNode<L> {
    fn refresh() -> Self::Message {
        Signal::Refresh(guinea_core::trace::current())
    }
}

/// A page as the reconciler sees it: the page's own state, and nothing else.
///
/// The component's `Input` is the segment's props, which the reconciler already
/// compares for us - `SegmentProps` has the `PartialEq` that decides whether
/// this subtree is still the same one.
pub struct PageNode<P> {
    page: Shared<P>,
    /// The segment's props, kept because `update` is not handed the input and
    /// a node that changes itself often needs to read what it is sitting in.
    props: SegmentProps<WinUi>,
}

impl<P: Page> Component for PageNode<P> {
    type Input = SegmentProps<WinUi>;
    type Message = Signal<P::Message>;

    fn create(input: &Self::Input, _cx: &ComponentContext<Self>) -> Self {
        #[cfg(feature = "harness")]
        {
            let sender = _cx.sender();
            crate::harness::remember::<P, P::Message>(move |signal| sender.send(signal));
        }

        Self {
            page: staged::<P>(input),
            props: input.clone(),
        }
    }

    /// The same page type in the same place, and either the same segment or
    /// one installed again - with other params, into a scope of its own. The
    /// second is a new node, and `install` staged it.
    fn input_changed(&mut self, input: &Self::Input, _cx: &ComponentContext<Self>) {
        if input.identity() != self.props.identity() {
            self.page = staged::<P>(input);
        }
        self.props = input.clone();
    }

    fn update(&mut self, message: Signal<P::Message>, cx: &ComponentContext<Self>) {
        match message {
            // Nothing to change. Delivering a message is itself what marks the
            // segment for redrawing - the pump pushes the token onto its dirty
            // list when it dispatches, not when something is mutated - so a
            // refresh that touches no state still brings the view round.
            Signal::Refresh(cause) => handling(cause),
            Signal::OpenWindow(window) => open(cx, window),
            Signal::Node(message) => self.page.borrow_mut().update(
                message,
                &mut UpdateCx {
                    props: &self.props,
                    segment: PhantomData,
                },
            ),
        }
    }

    fn view(&self, input: &Self::Input, cx: &mut ViewContext<Self>) -> View {
        // The root segment starts the frame the profiler collects; the ones
        // under it draw inside the same one.
        if input.cursor == 0 {
            guinea_core::observability::profiling::frame_done();
        }
        let _caused = caused(cx);
        let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
        leaves_its_fills(input, cx);
        crate::slots::drawing(input);
        let view = self.page.borrow().view(&mut PageCx {
            props: input.clone(),
            cx,
            page: PhantomData,
        });
        crate::slots::drawn(input);
        crate::devtools::record(input, &view);
        marked::<P>(view)
    }
}

pub struct LayoutNode<L> {
    layout: Shared<L>,
    props: SegmentProps<WinUi>,
}

impl<L: Layout> Component for LayoutNode<L> {
    type Input = SegmentProps<WinUi>;
    type Message = Signal<L::Message>;

    fn create(input: &Self::Input, _cx: &ComponentContext<Self>) -> Self {
        #[cfg(feature = "harness")]
        {
            let sender = _cx.sender();
            crate::harness::remember::<L, L::Message>(move |signal| sender.send(signal));
        }

        Self {
            layout: staged::<L>(input),
            props: input.clone(),
        }
    }

    /// See `PageNode::input_changed`.
    fn input_changed(&mut self, input: &Self::Input, _cx: &ComponentContext<Self>) {
        if input.identity() != self.props.identity() {
            self.layout = staged::<L>(input);
        }
        self.props = input.clone();
    }

    fn update(&mut self, message: Signal<L::Message>, cx: &ComponentContext<Self>) {
        match message {
            // See `PageNode::update`.
            Signal::Refresh(cause) => handling(cause),
            Signal::OpenWindow(window) => open(cx, window),
            Signal::Node(message) => self.layout.borrow_mut().update(
                message,
                &mut UpdateCx {
                    props: &self.props,
                    segment: PhantomData,
                },
            ),
        }
    }

    fn view(&self, input: &Self::Input, cx: &mut ViewContext<Self>) -> View {
        if input.cursor == 0 {
            guinea_core::observability::profiling::frame_done();
        }
        let _caused = caused(cx);
        let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
        leaves_its_fills(input, cx);
        crate::slots::drawing(input);
        let view = self.layout.borrow().view(&mut LayoutCx {
            props: input.clone(),
            cx,
            layout: PhantomData,
        });
        crate::slots::drawn(input);
        crate::devtools::record(input, &view);
        marked::<L>(view)
    }
}

thread_local! {
    static HANDLING: std::cell::Cell<Option<Cause>> = const { std::cell::Cell::new(None) };
    static DRAWN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Notes what the message being handled was sent under.
///
/// The reactor draws a drain after the send, where nothing is current. What it
/// draws for one message - the component it went to, and everything that
/// component expands into - is drawn before the effects of that render
/// commit, so the cause holds until [`caused`]'s effect clears it.
pub(crate) fn handling(cause: Option<Cause>) {
    HANDLING.set(cause);
}

/// Makes what this drawing is the doing of current for the guard's life: the
/// message being handled, or whatever is current already.
pub(crate) fn caused<C: Component>(cx: &mut ViewContext<C>) -> guinea_core::trace::Resumed {
    let cause = HANDLING.get().or_else(guinea_core::trace::current);

    let drawn = DRAWN.get().wrapping_add(1);
    DRAWN.set(drawn);
    cx.use_effect("guinea.caused", drawn, || {
        HANDLING.set(None);
        None
    });

    guinea_core::trace::resume(cause)
}

/// Withdraws what a segment filled, and the slots it placed, when it leaves
/// the screen or is installed again elsewhere.
fn leaves_its_fills<C: Component>(props: &SegmentProps<WinUi>, cx: &mut ViewContext<C>) {
    let filler = props.scopes[props.cursor].key();
    cx.use_effect("guinea.fills", props.identity(), move || {
        Some(Box::new(move || crate::slots::left(filler)) as Box<dyn FnOnce()>)
    });
}

/// A segment's view inside a border whose `AutomationId` is the segment's
/// name, so the XAML tree shows where each page and layout begins.
fn marked<S>(view: View) -> View {
    Border::new()
        .automation_id(guinea_router::observability::short(std::any::type_name::<S>()))
        .content(view)
        .into()
}

/// A window to open: a view as it is, or one that belongs to the application
/// it is opened from - a second window of the route tree, say.
pub struct NewWindow(Opens);

enum Opens {
    View(View),
    Under(Box<dyn FnOnce(&ScopeContext) -> View>),
}

impl NewWindow {
    /// A window built from the application it is opened from.
    pub fn under(build: impl FnOnce(&ScopeContext) -> View + 'static) -> Self {
        Self(Opens::Under(Box::new(build)))
    }

    fn open_from<C: Component>(self, cx: &mut ViewContext<C>) -> View {
        let router = cx.use_context(router_context());

        match self.0 {
            Opens::View(view) => view,
            Opens::Under(build) => {
                let router = router.unwrap_or_else(|| {
                    panic!(
                        "a window of the application is opened from inside the tree a \
                         RouterRoot renders - there is no router above this view"
                    )
                });
                let app = router.0.host().application().cloned().unwrap_or_else(|| {
                    panic!("the router above this view belongs to no application to open a window of")
                });
                build(&app)
            }
        }
    }
}

impl From<View> for NewWindow {
    fn from(view: View) -> Self {
        Self(Opens::View(view))
    }
}

fn open<C: Component>(cx: &ComponentContext<C>, window: View) {
    if !cx.open_window::<Shown>(window) {
        tracing::warn!("no active publication; the window was not opened");
    }
}

/// A window whose content is a view already built: the reactor opens windows
/// as components, and guinea hands it views. Held in a border, because a
/// component's view has to start with an element of its own.
pub(crate) struct Shown;

impl Component for Shown {
    type Input = View;
    type Message = ();

    fn create(_input: &View, _cx: &ComponentContext<Self>) -> Self {
        Self
    }

    fn view(&self, input: &View, _cx: &mut ViewContext<Self>) -> View {
        Border::new().content(input.clone()).into()
    }
}

/// A way to ask a segment to publish again, with its own message type erased.
///
/// Erased because nothing outside this crate should have to name
/// `PageNode<P>`: a plugin that redraws a view when the language or the theme
/// changes needs the same handle and knows nothing about pages.
#[derive(Clone)]
pub struct Refresher(Rc<dyn Fn()>);

impl Refresher {
    pub fn of<C: Refreshable>(cx: &ViewContext<C>) -> Self {
        let sender = cx.sender();
        Self(Rc::new(move || {
            let _gone = !sender.send(C::refresh());
        }))
    }

    /// What a subscription calls when the thing it watches has changed.
    pub fn refresh(&self) {
        (self.0)();
    }
}

/// What a node's `update` is handed.
///
/// Reading and acting, and nothing else. Navigation is not here on purpose: it
/// starts from a widget event, and the handle a view already has is captured
/// into the callback where the event is declared - which reads better than
/// fetching one again in the place the event arrives.
pub struct UpdateCx<'a, S> {
    props: &'a SegmentProps<WinUi>,
    segment: PhantomData<fn() -> S>,
}

impl<S> UpdateCx<'_, S> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// No subscription: `update` is a moment, not a view, and the segment is
    /// already publishing again because of the message that got here.
    pub fn read<R>(&self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
    {
        let binding = self.props.binding::<R>();
        (binding.get(), binding.dispatch())
    }
}

/// Reads a reducer's state, and asks this segment to publish again whenever it
/// changes.
///
/// The state is read fresh rather than mirrored into the node. A reducer lives
/// in the scope and changes from under the view, so a copy held here would be
/// one more thing to keep in step; the subscription says *when* to look, and
/// looking is `binding.get()`.
///
/// Taken again when the segment is installed again in the same place: the
/// one before listens to a scope that is gone.
fn use_reducer<R, C>(
    props: &SegmentProps<WinUi>,
    cx: &mut ViewContext<C>,
) -> (Rc<R>, guinea_core::feature::Dispatch)
where
    R: Reducer + PartialEq,
    C: Refreshable,
{
    let binding = props.binding::<R>();
    let refresher = Refresher::of(cx);

    // Keyed by the reducer, so a view reading two of them gets two
    // subscriptions rather than one that replaces the other.
    let watching = binding.clone();
    cx.use_effect(std::any::type_name::<R>(), props.identity(), move || {
        let subscription = watching.on_change(move |_| refresher.refresh());
        // Ends with this component instance rather than with the scope: one
        // that had been unmounted and kept listening would be asking a
        // reconciler slot nobody renders to publish.
        Some(Box::new(move || drop(subscription)) as Box<dyn FnOnce()>)
    });

    (binding.get(), binding.dispatch())
}

/// A context slot that is the same slot every time it is asked for.
///
/// `Context<T>` is a value now rather than an id, and `provide`/`use_context`
/// pair by identity, so the value has to outlive every render that uses it.
/// One per `T`, leaked on first use, keyed by type - which is what the old
/// `ContextId` cache did before the ids became private.
fn context_for<T>(default: fn() -> T) -> &'static windows_reactor::Context<T>
where
    T: 'static,
{
    thread_local! {
        static SLOTS: RefCell<HashMap<TypeId, &'static (dyn Any + 'static)>> =
            RefCell::new(HashMap::new());
    }

    SLOTS.with(|slots| {
        let slot = *slots
            .borrow_mut()
            .entry(TypeId::of::<T>())
            .or_insert_with(|| {
                let context: &'static windows_reactor::Context<T> =
                    Box::leak(Box::new(windows_reactor::Context::new(default())));
                context as &'static (dyn Any + 'static)
            });

        slot.downcast_ref::<windows_reactor::Context<T>>()
            .expect("the slot is keyed by the type it holds")
    })
}

pub(crate) fn nav_context<R: 'static>()
-> &'static windows_reactor::Context<Option<NavigateHandle<WinUi, R>>> {
    context_for::<Option<NavigateHandle<WinUi, R>>>(|| None)
}

pub(crate) fn route_context<R: 'static>() -> &'static windows_reactor::Context<Option<R>> {
    context_for::<Option<R>>(|| None)
}

/// A router in a context slot. Compared by identity - a `Router` has no
/// meaningful equality, and two handles mean the same router or a different
/// one.
#[derive(Clone)]
pub struct RouterHandle(Rc<Router<WinUi>>);

impl PartialEq for RouterHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

fn router_context() -> &'static windows_reactor::Context<Option<RouterHandle>> {
    context_for::<Option<RouterHandle>>(|| None)
}

/// The route tree as a component, and the root of a window that has one.
///
/// It owns the router, which is what makes a window a root: `Router::new` opens
/// a `FeatureHost`, and the host's registration is what publishes `RootOpened`
/// and `RootClosed`. Both now happen where they should - when the window's
/// component is created and dropped - rather than inside a render hook, which
/// is where the old shape had to put them.
pub struct RouterRoot<R: RouteChain<WinUi> + Clone + PartialEq + 'static> {
    router: Rc<Router<WinUi>>,
    route: R,
    /// Why the first route is not standing, when it failed to install.
    failure: Option<String>,
    _question: guinea_router::router::RouteHookHandle,
    _panel: guinea_core::observability::panels::PanelGuard,
}

/// What a [`RouterRoot`] is opened with: the application whose window it is,
/// and where it starts.
#[derive(Clone)]
pub struct Rooted<R> {
    pub app: ScopeContext,
    pub initial: R,
}

/// The same application, at the same route.
impl<R: PartialEq> PartialEq for Rooted<R> {
    fn eq(&self, other: &Self) -> bool {
        self.app.scope == other.app.scope && self.initial == other.initial
    }
}

/// What reaches a [`RouterRoot`].
pub enum Routed<R> {
    /// The router is here now. `NavigateHandle` publishes through the sink
    /// the root hands out, once the navigation has happened - with what set
    /// it off, which the segments it draws are the doing of.
    Arrived(R, Option<Cause>),
    /// A guard's question came or went. See [`Router::pending`].
    Asked,
}

impl<R> Component for RouterRoot<R>
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    /// The application, and where the window starts. A second window opened
    /// with a different route is a different input, and gets its own router.
    type Input = Rooted<R>;
    type Message = Routed<R>;

    /// Installs the first route before the first view. One that fails leaves
    /// the window saying why; the main window's failure also ends the
    /// application, and `run` returns it.
    fn create(input: &Rooted<R>, cx: &ComponentContext<Self>) -> Self {
        let initial = &input.initial;
        let router = Rc::new(Router::new(guinea_app::feature::FeatureHost::under(&input.app)));

        #[cfg(feature = "harness")]
        crate::harness::remember_router(&router);

        crate::run::standing(&router);
        let main = guinea_app::app::roots::labelled(crate::run::MAIN).is_none();
        if main {
            guinea_app::app::roots::set_label(router.root(), crate::run::MAIN);
        }

        let sender = cx.sender();
        let question = router.on_question(move || {
            let _gone = !sender.send(Routed::Asked);
        });

        let failure = router.navigate(initial.clone()).err().map(|error| {
            let shown = format!("{error:#}");
            tracing::error!(error = %shown, route = initial.name(), "the first route did not install");
            if main {
                crate::run::failed(error);
            }
            shown
        });

        Self {
            _panel: crate::devtools::offer(&router),
            _question: question,
            failure,
            router,
            route: initial.clone(),
        }
    }

    /// Takes the route the router arrived at, and moves nothing: the
    /// navigation has already happened, guards and all.
    fn update(&mut self, message: Routed<R>, _cx: &ComponentContext<Self>) {
        match message {
            Routed::Arrived(route, cause) => {
                handling(cause);
                self.route = route;
            }
            Routed::Asked => {}
        }
    }

    fn view(&self, _input: &Rooted<R>, cx: &mut ViewContext<Self>) -> View {
        let _caused = caused(cx);

        let sender = cx.sender();
        let nav = NavigateHandle::new(
            self.router.clone(),
            RouteSink::new(move |route: R| {
                let _gone = !sender.send(Routed::Arrived(route, guinea_core::trace::current()));
            }),
        );

        #[cfg(feature = "harness")]
        crate::harness::remember_nav(&nav);

        let tree: View = match (self.router.active_chain(), &self.failure) {
            (Some(_), _) => self.router.render(&()),
            (None, Some(failure)) => TextBlock::new().text(failure.clone()).into(),
            (None, None) => Grid::new().into(),
        };
        let tree = provide(
            router_context(),
            Some(RouterHandle(self.router.clone())),
            tree,
        );
        let tree = provide(route_context::<R>(), Some(self.route.clone()), tree);
        let tree = provide(nav_context::<R>(), Some(nav), tree);

        Grid::new()
            .children((tree,))
            .content_dialog(question(&self.router))
    }
}

/// The question a guard is waiting on, as a dialog whose buttons answer it.
///
/// Always attached to the root and shown only while there is a question, so
/// the route's own tree keeps its place whether one is asked or not.
fn question(router: &Rc<Router<WinUi>>) -> ContentDialog {
    let pending = router.pending();
    let open = pending.is_some();
    let ask = pending.unwrap_or_else(|| guinea_core::guard::Ask::new("", "", ""));

    let answering = Rc::downgrade(router);
    ContentDialog::new()
        .is_open(open)
        .primary_button_text(ask.confirm)
        .close_button_text(ask.cancel)
        .on_closed(move |result: ContentDialogResult| {
            if let Some(router) = answering.upgrade() {
                router.answer(result == ContentDialogResult::Primary);
            }
        })
        .content(TextBlock::new().text(ask.text))
}

/// Subscribing to route changes from a view, the way any other effect is
/// written: registered once, undone when the view unmounts.
pub trait UseRouteChange {
    /// Runs `hook` after each navigation, for as long as this view is mounted.
    /// Panics if there is no router above - a wiring mistake, not a state a
    /// running application can reach.
    fn use_route_change(&mut self, hook: impl Fn(Option<&str>, &str) + 'static);
}

impl<C: Component> UseRouteChange for ViewContext<'_, C> {
    fn use_route_change(&mut self, hook: impl Fn(Option<&str>, &str) + 'static) {
        let router = self.use_context(router_context()).unwrap_or_else(|| {
            panic!(
                "use_route_change() found no router above this view - it has to be \
                 called from inside the tree a RouterRoot renders"
            )
        });

        self.use_effect("guinea::route_change", (), move || {
            let handle = router.0.on_route_change(hook);
            Some(Box::new(move || drop(handle)) as Box<dyn FnOnce()>)
        });
    }
}

pub trait UseNavigate {
    fn use_navigate<R>(&mut self) -> NavigateHandle<WinUi, R>
    where
        R: RouteChain<WinUi> + Clone + PartialEq + 'static;
}

impl<C: Component> UseNavigate for ViewContext<'_, C> {
    fn use_navigate<R>(&mut self) -> NavigateHandle<WinUi, R>
    where
        R: RouteChain<WinUi> + Clone + PartialEq + 'static,
    {
        self.use_context(nav_context::<R>()).unwrap_or_else(|| {
            panic!(
                "use_navigate::<{}>() called with no NavigateHandle provided - \
                 render the tree through RouterRoot::<{0}>",
                std::any::type_name::<R>()
            )
        })
    }
}

pub trait UseRoute {
    fn use_route<R>(&mut self) -> R
    where
        R: Clone + PartialEq + 'static;
}

impl<C: Component> UseRoute for ViewContext<'_, C> {
    fn use_route<R>(&mut self) -> R
    where
        R: Clone + PartialEq + 'static,
    {
        self.use_context(route_context::<R>()).unwrap_or_else(|| {
            panic!(
                "use_route::<{}>() called with no route provided - \
                 render the tree through RouterRoot::<{0}>",
                std::any::type_name::<R>()
            )
        })
    }
}

/// What a page's view is handed.
///
/// Carries the page type, not because rendering needs it, but because reading
/// does: what a segment may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, 'v, P: Page> {
    props: SegmentProps<WinUi>,
    cx: &'a mut ViewContext<'v, PageNode<P>>,
    page: PhantomData<fn() -> P>,
}

impl<P: Page> PageCx<'_, '_, P> {
    /// Seals a widget's event as one of this page's own messages.
    ///
    /// The seam, and the whole reason a parent never names a child's message
    /// type: what leaves this page is a `Callback` the reconciler understands,
    /// and what goes in is the page's own enum.
    pub fn on<T>(&self, message: impl Fn(T) -> P::Message + 'static) -> Callback<T>
    where
        T: 'static,
    {
        self.cx.callback(move |payload| Signal::Node(message(payload)))
    }

    /// Opens `window` as a window of its own - see [`crate::window`].
    ///
    /// Deferred rather than immediate, and the deferral is the reactor's rule
    /// rather than ours: a window may be opened during `create`, `changed` or
    /// `update`, never during `view`.
    pub fn open_window(&mut self, window: impl Into<NewWindow>) -> Callback<()> {
        let window = window.into().open_from(self.cx);
        let sender = self.cx.sender();
        let window = RefCell::new(Some(window));
        Callback::new(move |_| {
            if let Some(window) = window.borrow_mut().take()
                && !sender.send(Signal::OpenWindow(window))
            {
                tracing::warn!("the segment asking for a window is gone; not opened");
            }
        })
    }

    /// A navigator over the route type the application runs.
    pub fn navigate<R>(&mut self) -> NavigateHandle<WinUi, R>
    where
        R: RouteChain<WinUi> + Clone + PartialEq + 'static,
    {
        self.cx.use_navigate::<R>()
    }

    /// Shows `view` in the slot `S` the nearest layout above placed. See
    /// [`LayoutCx::slot`].
    ///
    /// The innermost fill on the active chain wins. A fill is made again on
    /// every draw, and one not made again is withdrawn, as is everything a
    /// segment filled when it leaves. Callbacks in `view` belong to this
    /// segment, so a toolbar button in the layout acts on the page.
    ///
    /// Filling a slot no layout above placed panics in a debug build.
    pub fn fill<S: Slot>(&mut self, view: View) {
        crate::slots::fill::<S>(&self.props, view);
    }
}

impl<P: Page> PageCx<'_, '_, P> {
    /// Reads a reducer's state, and asks this segment to publish again when it
    /// changes.
    ///
    /// The feature that answers is the one this page installed, or the nearest
    /// above that listed `R` in `Exports`. A read that reaches nothing panics
    /// here, naming the chain it walked.
    pub fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(&self.props, self.cx)
    }
}

impl<P: Page> Reads for PageCx<'_, '_, P> {
    fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(&self.props, self.cx)
    }

    fn dispatch<R>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
    {
        self.props.binding::<R>().dispatch()
    }
}

impl<'v, P: Page> std::ops::Deref for PageCx<'_, 'v, P> {
    type Target = ViewContext<'v, PageNode<P>>;
    fn deref(&self) -> &Self::Target {
        self.cx
    }
}

impl<P: Page> std::ops::DerefMut for PageCx<'_, '_, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}

pub struct LayoutCx<'a, 'v, L: Layout> {
    props: SegmentProps<WinUi>,
    cx: &'a mut ViewContext<'v, LayoutNode<L>>,
    layout: PhantomData<fn() -> L>,
}

impl<L: Layout> LayoutCx<'_, '_, L> {
    /// See [`PageCx::on`].
    pub fn on<T>(&self, message: impl Fn(T) -> L::Message + 'static) -> Callback<T>
    where
        T: 'static,
    {
        self.cx.callback(move |payload| Signal::Node(message(payload)))
    }

    /// The next segment down the chain, for the layout to place where it wants.
    pub fn outlet(&mut self) -> View {
        self.props.outlet(&())
    }

    /// Whether the segment directly below is `P` - what a tab strip needs to
    /// highlight the current tab without keeping a copy of the route.
    pub fn child_is<P: 'static>(&self) -> bool {
        self.props
            .chain
            .get(self.props.cursor + 1)
            .is_some_and(|entry| (entry.type_id)() == TypeId::of::<P>())
    }

    /// See [`PageCx::open_window`].
    pub fn open_window(&mut self, window: impl Into<NewWindow>) -> Callback<()> {
        let window = window.into().open_from(self.cx);
        let sender = self.cx.sender();
        let window = RefCell::new(Some(window));
        Callback::new(move |_| {
            if let Some(window) = window.borrow_mut().take()
                && !sender.send(Signal::OpenWindow(window))
            {
                tracing::warn!("the segment asking for a window is gone; not opened");
            }
        })
    }

    /// A navigator over the route type the application runs.
    pub fn navigate<R>(&mut self) -> NavigateHandle<WinUi, R>
    where
        R: RouteChain<WinUi> + Clone + PartialEq + 'static,
    {
        self.cx.use_navigate::<R>()
    }

    /// Places the slot `S`: what the innermost segment below filled it with,
    /// or nothing.
    ///
    /// Placing it is what declares it. The fill arrives a drain after the
    /// segment that made it drew, and redraws the slot alone - not this
    /// layout, and not the outlet.
    pub fn slot<S: Slot>(&mut self) -> View {
        crate::slots::place::<S>(&self.props)
    }

    /// See [`PageCx::fill`]. A layout fills a slot a layout above placed.
    pub fn fill<S: Slot>(&mut self, view: View) {
        crate::slots::fill::<S>(&self.props, view);
    }

    /// This layout's children in `routes!`, each as the route that reaches it
    /// from here, and which one is showing - what a menu is built from.
    ///
    /// Labels belong in an exhaustive `match` on `R` in the view, so a page
    /// added to the tree and left out of the menu does not compile. A child
    /// that needs more than this layout carries is not offered.
    pub fn child_routes<R>(&mut self) -> Vec<guinea_router::router::ChildRoute<R>>
    where
        R: RouteChain<WinUi> + 'static,
    {
        match self.cx.use_context(router_context()) {
            Some(router) => router.0.child_routes::<R>(self.props.cursor),
            None => Vec::new(),
        }
    }
}

impl<L: Layout> LayoutCx<'_, '_, L> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(&self.props, self.cx)
    }
}

impl<L: Layout> Reads for LayoutCx<'_, '_, L> {
    fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(&self.props, self.cx)
    }

    fn dispatch<R>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
    {
        self.props.binding::<R>().dispatch()
    }
}

impl<'v, L: Layout> std::ops::Deref for LayoutCx<'_, 'v, L> {
    type Target = ViewContext<'v, LayoutNode<L>>;
    fn deref(&self) -> &Self::Target {
        self.cx
    }
}

impl<L: Layout> std::ops::DerefMut for LayoutCx<'_, '_, L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}
