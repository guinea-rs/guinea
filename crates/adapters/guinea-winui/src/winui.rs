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

use std::any::TypeId;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_core::guard::Verdict;
use guinea_core::scope::{Reducer, Scope};
use guinea_core::trace::Cause;

use guinea_app::app::AppContext;
use guinea_app::feature::{FeatureInitContext, Install};
use guinea_router::mount::{Outlet, Pass, Reads, binding, navigator};
use guinea_router::navigator::{Navigator, Routable};
use guinea_router::slot::Slot;
use windows_reactor::{
    Border, Callback, Component, ComponentContext, Grid, TextBlock, View, ViewContext,
};

/// A leaf of the route tree, and an Elm node.
///
/// The struct that implements this is the page's state. Nothing above it ever
/// names its `Message`, so adding a page costs no edit anywhere else.
///
/// Each part has one job:
///
/// - [`install`](Page::install) mounts the page: it sets up what the page
///   needs while it is mounted - the features it installs with
///   `cx.install::<F>(..)` - and returns the node it starts as. Both live in
///   the page's scope and go when the page is left.
/// - [`update`](Page::update) is the only place the page's own state changes,
///   one [`Message`](Page::Message) at a time.
/// - [`view`](Page::view) draws that state. It reads what features publish
///   with [`read`](PageCx::read), turns a widget's event into a
///   message with [`on`](PageCx::on), and asks a feature for something with
///   `emit` on the dispatch it was handed.
///
/// `#[page]` writes what a page leaves out: `Params = ()` for a page that
/// captures nothing, an `install` that mounts the `Default` node and installs
/// nothing, and a
/// `Message` nobody can send with its empty `update`. That `install` is the
/// only thing that asks for `Default`: a page that writes its own is made
/// there, and need not have one. The smallest page is a view:
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
/// use guinea_winui::{Install, Page, PageCx, UpdateCx, page};
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
/// fn counter(cx: &Install) -> anyhow::Result<Counter> {
///     let count = cx.state::<Count>().plain();
///
///     cx.answers(move |Add(by): Add| count.push(by));
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
///     type Message = Msg;
///
///     fn install(ctx: &Install) -> anyhow::Result<Self> {
///         ctx.install::<Counter>(&())?;
///         Ok(Self::default())
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
pub trait Page: Sized + 'static {
    /// Where `impl Page` was written, for devtools to link to. `#[page]`
    /// fills it in; an impl without it loses only the source link.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the page is mounted with: `cx.mount::<ProcessPage>(&pid)`. `()`
    /// for a page that needs nothing, which `#[page]` writes when it is left
    /// out.
    ///
    /// `PartialEq` because a mount's one question about it is whether it is
    /// still the same: equal params are the same page in the same scope,
    /// other params another page in a scope of its own.
    ///
    /// A page sees it in [`install`](Self::install), for what the features
    /// need and for what the node keeps.
    type Params: PartialEq + Clone + 'static;

    /// This page's own events. An enum, and nobody else's business.
    ///
    /// Made from a widget's event by [`PageCx::on`], and handled in
    /// [`update`](Self::update). A page with nothing to handle leaves it out,
    /// and `#[page]` writes one that cannot be sent along with the empty
    /// `update`.
    type Message: 'static;

    /// Mounts the page: installs what it needs while it is mounted, and
    /// returns the node it starts as.
    ///
    /// Runs once per scope, when the scope is made, and in it: what it
    /// installs goes when the page is left, and the node lives as long as the
    /// scope. The page reads what it installed itself, and what a segment
    /// above lists in `Exports`. A page woken from sleep is not mounted again. Everything an
    /// effect needs exactly once per mount belongs here - a first `Refresh`
    /// emitted to a feature, say - and so does anything the node starts from.
    ///
    /// <!-- shown: a page that installs -->
    /// ```rust,ignore
    /// use guinea_winui::{Install, Page, PageCx, page};
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
    /// fn processes(cx: &Install<str>) -> anyhow::Result<Processes> {
    ///     let listing = cx.state::<Listing>().plain();
    ///     listing.push(format!("processes on {}", cx.params()));
    ///     Ok(Processes(listing))
    /// }
    ///
    /// #[derive(Clone, PartialEq)]
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
    ///
    ///     fn install(ctx: &Install<ProcessesParams>) -> anyhow::Result<Self> {
    ///         ctx.install::<Processes>(&ctx.params().context)?;
    ///         ctx.state::<Selection>().plain();
    ///         Ok(Self)
    ///     }
    ///
    ///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         let (listing, _) = cx.read::<Listing>();
    ///         TextBlock::new().text(listing.0.clone()).into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    ///
    /// Where a page keeps what it was reached with, since `view` is not
    /// handed the capture:
    ///
    /// <!-- shown: a page that keeps its capture -->
    /// ```rust,ignore
    /// use guinea_winui::{Install, Page, PageCx, page};
    /// use windows_reactor::{TextBlock, View};
    ///
    /// #[derive(Clone, PartialEq)]
    /// pub struct ProcessParams {
    ///     pub pid: u32,
    /// }
    ///
    /// pub struct ProcessPage {
    ///     pid: u32,
    /// }
    ///
    /// #[page]
    /// impl Page for ProcessPage {
    ///     type Params = ProcessParams;
    ///
    ///     fn install(ctx: &Install<ProcessParams>) -> anyhow::Result<Self> {
    ///         Ok(Self { pid: ctx.params().pid })
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
    ///
    /// And where it starts from something read while it mounts - what the
    /// application saved, say - and is its own from then on:
    ///
    /// <!-- shown: a page that starts from a setting -->
    /// ```rust,ignore
    /// use guinea_winui::{Install, Page, PageCx, UpdateCx, page};
    /// use windows_reactor::{Button, StackPanel, TextBlock, View};
    ///
    /// /// Column widths as the application saved them; a default when it did not.
    /// #[derive(Clone)]
    /// pub struct SavedWidths(pub Vec<u32>);
    ///
    /// impl Default for SavedWidths {
    ///     fn default() -> Self {
    ///         Self(vec![280, 80])
    ///     }
    /// }
    ///
    /// /// The widths are the page's own once it is mounted: a drag changes them
    /// /// here, and nothing else needs to know. No `Default`: `install` is the
    /// /// one place a table is made.
    /// pub struct Table {
    ///     widths: Vec<u32>,
    /// }
    ///
    /// pub enum Msg {
    ///     Widen,
    /// }
    ///
    /// #[page]
    /// impl Page for Table {
    ///     type Message = Msg;
    ///
    ///     fn install(_ctx: &Install, saved: Option<SavedWidths>) -> anyhow::Result<Self> {
    ///         Ok(Self { widths: saved.unwrap_or_default().0 })
    ///     }
    ///
    ///     fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
    ///         match message {
    ///             Msg::Widen => self.widths[0] += 20,
    ///         }
    ///     }
    ///
    ///     fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
    ///         StackPanel::new()
    ///             .children((
    ///                 TextBlock::new().text(format!("widths {:?}", self.widths)),
    ///                 Button::new()
    ///                     .on_click(cx.on(|()| Msg::Widen))
    ///                     .content(TextBlock::new().text("Widen")),
    ///             ))
    ///             .into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn install(cx: &Install<Self::Params>) -> anyhow::Result<Self>;

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
    /// use guinea_winui::{Install, Page, PageCx, UpdateCx, page};
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
    /// fn searching(cx: &Install) -> anyhow::Result<Searching> {
    ///     let results = cx.state::<Results>().plain();
    ///
    ///     cx.answers(move |Search(query): Search| results.push(format!("found {query}")));
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
    ///     type Message = Msg;
    ///
    ///     fn install(ctx: &Install) -> anyhow::Result<Self> {
    ///         ctx.install::<Searching>(&())?;
    ///         Ok(Self::default())
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
    ///                 TextBox::new().text(self.name.clone()).on_text_changed(cx.on(Msg::Named)),
    ///                 TextBlock::new().text(format!("hello, {}", self.name)),
    ///             ))
    ///             .into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View;
}

/// A branch: an Elm node that also mounts what goes under it.
///
/// Everything a [`Page`] is, plus two things only a branch has:
///
/// - It mounts: [`mount`](LayoutCx::mount) puts a page or a layout under it,
///   in a scope of its own, and [`outlet`](LayoutCx::outlet) mounts whatever
///   a navigator says is there. Either is a view, placed wherever the layout
///   wants - beside a sidebar, under a tab strip.
/// - What it installs outlives what it mounts. A feature a layout installs is
///   there for every page below and stays while they come and go, and
///   whatever it lists in `Exports` those pages may read.
///
/// `#[layout]` writes what a layout leaves out, the same as `#[page]`.
///
/// A shell with a sidebar it can close, and a tab it marks while its page is
/// the one showing:
///
/// <!-- shown: a shell -->
/// ```rust,ignore
/// use guinea_winui::{
///     Install, Layout, LayoutCx, Page, PageCx, Routable, UpdateCx, layout, page,
/// };
/// use windows_reactor::{Border, Button, StackPanel, TextBlock, View};
///
/// /// Where the shell can be.
/// #[derive(Clone, PartialEq, Debug, Routable)]
/// pub enum Place {
///     Home,
///     About,
/// }
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
/// fn chrome(cx: &Install) -> anyhow::Result<Chrome> {
///     let sidebar = cx.state::<Sidebar>().seed(Sidebar { open: true }).plain();
///
///     cx.answers(move |SetOpen(open): SetOpen| sidebar.push(open));
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
///         // its first render, naming what it was mounted under.
///         let (sidebar, _) = cx.read::<Sidebar>();
///         let width = if sidebar.open { "narrow" } else { "wide" };
///
///         TextBlock::new().text(format!("home, {width}")).into()
///     }
/// }
///
/// #[derive(Default)]
/// pub struct About;
///
/// #[page]
/// impl Page for About {
///     fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
///         TextBlock::new().text("guinea").into()
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
///     type Message = ShellMsg;
///
///     /// A feature, and a reducer claimed directly. Pages below may read
///     /// `Sidebar`, which `Chrome` exports; `Title` is the shell's own.
///     fn install(ctx: &Install) -> anyhow::Result<Self> {
///         ctx.install::<Chrome>(&())?;
///         ctx.state::<Title>().seed(Title("guinea".into())).plain();
///         Ok(Self)
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
///         let nav = cx.navigate::<Place>();
///
///         let tab = match nav.current() {
///             Some(Place::Home) => "> Home",
///             _ => "Home",
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
///                 cx.outlet(&nav, |place, cx| match place {
///                     Place::Home => cx.mount::<Home>(&()),
///                     Place::About => cx.mount::<About>(&()),
///                 }),
///             ))
///             .into()
///     }
/// }
/// ```
/// <!-- /shown -->
///
/// Nothing reaches up past what is above it. A page that no layout above
/// installed `Sidebar` for panics on its first render, naming what it was
/// mounted under and who claimed `Sidebar` without exporting it.
pub trait Layout: Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the layout is mounted with, as a page is: see [`Page::Params`].
    /// `()` when it is left out.
    ///
    /// The layout above mounting it with the same value keeps it, and
    /// everything it installed, where it is; another value mounts another
    /// one. That is what makes a feature per machine right to install here:
    ///
    /// <!-- shown: a layout per machine -->
    /// ```rust,ignore
    /// use guinea_winui::{Install, Layout, LayoutCx, layout};
    /// use windows_reactor::View;
    ///
    /// #[derive(Default)]
    /// pub struct MachineLayout;
    ///
    /// #[layout]
    /// impl Layout for MachineLayout {
    ///     /// Which machine: `cx.mount::<MachineLayout>(&machine)`.
    ///     type Params = String;
    ///
    ///     fn install(ctx: &Install<String>) -> anyhow::Result<Self> {
    ///         ctx.install::<Connection>(ctx.params())?;
    ///         Ok(Self)
    ///     }
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         cx.mount::<Overview>(&()).into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    type Params: PartialEq + Clone + 'static;

    /// This layout's own events. See [`Page::Message`].
    type Message: 'static;

    /// Mounts the layout: installs what it, and every page under it, needs
    /// while it is mounted, and returns the node it starts as. See
    /// [`Page::install`].
    ///
    /// What it installs is what the pages below share: they come and go, and
    /// it stays. Whatever its features list in `Exports` those pages may
    /// read; a reducer it claims directly is its own.
    ///
    /// <!-- shown: a layout that installs -->
    /// ```rust,ignore
    /// use guinea_winui::{Install, Layout, LayoutCx, layout};
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
    ///     /// `refresh` is a setting the application may `provide`.
    ///     fn install(ctx: &Install, refresh: Option<Refresh>) -> anyhow::Result<Self> {
    ///         let refresh = refresh.unwrap_or_default();
    ///
    ///         ctx.install::<SidebarFeature>(&())?;
    ///         ctx.install::<MetricsFeature>(&())?;
    ///         ctx.state::<Title>()
    ///             .seed(Title(format!("every {} ms", refresh.every_ms)))
    ///             .plain();
    ///
    ///         Ok(Self)
    ///     }
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         cx.mount::<MetricsPage>(&()).into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    ///
    /// Runs when the layout is mounted, which is not every navigation: a
    /// move between two of its pages keeps the node, and what the layout
    /// holds survives it.
    fn install(cx: &Install<Self::Params>) -> anyhow::Result<Self>;

    /// Asked before this layout is left. See [`Page::leaving`].
    ///
    /// Only when the layout itself goes: a navigation that changes the
    /// variant it was mounted for. One that moves only below it - between two
    /// of its own pages - asks the page, not the layout; when both would go,
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
    ///     fn leaving(&self) -> Verdict {
    ///         if self.step == 0 || self.finished {
    ///             Verdict::Allow
    ///         } else {
    ///             Verdict::ask(Ask::new("Abandon the setup?", "Abandon", "Continue"))
    ///         }
    ///     }
    ///
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         let nav = cx.navigate::<Step>();
    ///         cx.outlet(&nav, |step, cx| cx.mount::<StepPage>(step))
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

    /// Draws the layout around what it mounts. See [`Page::view`].
    ///
    /// What [`mount`](LayoutCx::mount) and [`outlet`](LayoutCx::outlet) give
    /// back is a value like any other view: placed once, beside or under
    /// whatever the layout draws itself, and left out for a layout that hides
    /// its content. The navigator says where the window is, for a tab strip
    /// that marks the current tab without keeping a copy of the route:
    ///
    /// <!-- shown: a tab strip -->
    /// ```rust,ignore
    /// use guinea_winui::{Layout, LayoutCx, Routable, layout};
    /// use windows_reactor::{StackPanel, TextBlock, View};
    ///
    /// #[derive(Clone, PartialEq, Debug, Routable)]
    /// pub enum Tab {
    ///     Processes,
    ///     Services,
    /// }
    ///
    /// #[derive(Default)]
    /// pub struct Tabs;
    ///
    /// #[layout]
    /// impl Layout for Tabs {
    ///     fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
    ///         let nav = cx.navigate::<Tab>();
    ///         let here = nav.current();
    ///         let tab = |name: &str, tab: Tab| {
    ///             TextBlock::new().text(if here == Some(tab) {
    ///                 format!("[{name}]")
    ///             } else {
    ///                 name.to_string()
    ///             })
    ///         };
    ///
    ///         StackPanel::new()
    ///             .children((
    ///                 tab("Processes", Tab::Processes),
    ///                 tab("Services", Tab::Services),
    ///                 cx.outlet(&nav, |tab, cx| match tab {
    ///                     Tab::Processes => cx.mount::<Processes>(&()),
    ///                     Tab::Services => cx.mount::<Services>(&()),
    ///                 }),
    ///             ))
    ///             .into()
    ///     }
    /// }
    /// ```
    /// <!-- /shown -->
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View;
}

/// A page or a layout: what [`LayoutCx::mount`] mounts. `#[page]` and
/// `#[layout]` write it.
#[doc(hidden)]
pub trait Mountable: 'static {
    type Params: PartialEq + Clone + 'static;
    type Message: 'static;

    /// Installs it into the scope `cx` is for, and keeps its node there and
    /// how it draws.
    fn install(cx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<()>;
}

/// The view of whatever was mounted into a scope, kept there as it mounts:
/// what an outlet draws for a child it holds without mounting it again.
struct Draws(fn(Scope) -> View);

/// Keeps the node `install` built in its scope, for the component to find.
///
/// `install` needs the feature context, which exists while the layout above
/// is mounting; a component is created later, when the reconciler reaches it,
/// and is handed no such thing. So the node is built where the context is and
/// found where it is needed.
///
/// Kept for as long as the scope is, not taken once: a component the
/// reconciler drops and makes again in the same scope is the same node.
fn stage<N: 'static>(ctx: &FeatureInitContext, node: Shared<N>) {
    ctx.__scope().keep(node);
}

/// The node installed into `scope`, and `None` once the scope is gone - the
/// component made for a scope removed while it was on its way draws nothing,
/// as [`left`] says.
fn staged<N: 'static>(scope: Scope) -> Option<Shared<N>> {
    scope.kept::<Shared<N>, _>(|node| node.clone())
}

#[doc(hidden)]
pub fn install_page<P: Page>(ctx: &FeatureInitContext, params: &P::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Draws(page_view::<P>));
    guinea_router::mount::written(scope, P::DECLARED);
    let node = P::install(&Install::new(ctx, params))?;
    stage_node(ctx, node, P::leaving);
    Ok(())
}

fn page_view<P: Page>(scope: Scope) -> View {
    View::component::<PageNode<P>>(scope)
}

fn layout_view<L: Layout>(scope: Scope) -> View {
    View::component::<LayoutNode<L>>(scope)
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

#[doc(hidden)]
pub fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &L::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Draws(layout_view::<L>));
    guinea_router::mount::written(scope, L::DECLARED);
    let node = L::install(&Install::new(ctx, params))?;
    stage_node(ctx, node, L::leaving);
    Ok(())
}

/// What a layout mounted: the child's view, or why it did not mount.
///
/// Drawn as it is - `cx.mount::<Services>(&()).into()` - or handed to an
/// [`outlet`](LayoutCx::outlet). One that failed draws what went wrong in
/// its place, and the others beside it stand.
#[must_use = "a mounted child shows only where its view is placed"]
pub struct Child {
    pass: Pass,
    mounted: Result<Scope, anyhow::Error>,
}

impl Child {
    /// Keeps the child asleep when a later draw leaves it out, instead of
    /// letting it go: it comes back as it was left.
    pub fn keep(self) -> Self {
        if let Ok(scope) = &self.mounted {
            self.pass.keep(*scope);
        }
        self
    }

    /// Why it did not mount, when it did not.
    pub fn failure(&self) -> Option<&anyhow::Error> {
        self.mounted.as_ref().err()
    }
}

impl From<Child> for View {
    fn from(child: Child) -> Self {
        match child.mounted {
            Ok(scope) => drawn(scope),
            Err(error) => failed(&error),
        }
    }
}

/// The view of what was mounted into `scope`.
pub(crate) fn drawn(scope: Scope) -> View {
    match scope.try_kept::<Draws, _>(|draws| draws.0) {
        Some(view) => view(scope),
        None => Grid::new().into(),
    }
}

fn failed(error: &anyhow::Error) -> View {
    TextBlock::new().text(format!("{error:#}")).into()
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
/// The component's `Input` is the scope the page was installed into: the
/// same scope is the same page, and another one - the same page type
/// mounted with other params - is a new node, which `install` staged there.
pub struct PageNode<P> {
    /// `None` when the scope was removed before the reconciler made this.
    page: Option<Shared<P>>,
    /// The scope, kept because `update` is not handed the input and a node
    /// that changes itself often needs to read what it is sitting in.
    scope: Scope,
    /// The reducers the render under way listens to, kept between renders
    /// so a render does not allocate a list of its own.
    subscribed: RefCell<Vec<TypeId>>,
}

impl<P: Page> Component for PageNode<P> {
    type Input = Scope;
    type Message = Signal<P::Message>;

    fn create(scope: &Scope, _cx: &ComponentContext<Self>) -> Self {
        #[cfg(feature = "harness")]
        {
            let sender = _cx.sender();
            crate::harness::remember::<P, P::Message>(move |signal| sender.send(signal));
        }

        Self {
            page: staged::<P>(*scope),
            scope: *scope,
            subscribed: RefCell::default(),
        }
    }

    fn input_changed(&mut self, scope: &Scope, _cx: &ComponentContext<Self>) {
        if *scope != self.scope {
            self.page = staged::<P>(*scope);
            self.scope = *scope;
        }
    }

    fn update(&mut self, message: Signal<P::Message>, cx: &ComponentContext<Self>) {
        match message {
            // Nothing to change. Delivering a message is itself what marks the
            // node for redrawing - the pump pushes the token onto its dirty
            // list when it dispatches, not when something is mutated - so a
            // refresh that touches no state still brings the view round.
            Signal::Refresh(cause) => handling(cause),
            Signal::OpenWindow(window) => open(cx, window),
            Signal::Node(message) => {
                let Some(page) = self.page.as_ref().filter(|_| !left(self.scope)) else {
                    return;
                };
                page.borrow_mut().update(
                    message,
                    &mut UpdateCx {
                        scope: self.scope,
                        node: PhantomData,
                    },
                )
            }
        }
    }

    fn view(&self, scope: &Scope, cx: &mut ViewContext<Self>) -> View {
        let scope = *scope;
        let Some(page) = self.page.as_ref().filter(|_| !left(scope)) else {
            return marked::<P>(Grid::new().into());
        };
        let _caused = caused(cx);
        let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
        leaves_its_fills(scope, cx);
        crate::slots::drawing(scope);

        let mut subscribed = self.subscribed.borrow_mut();
        subscribed.clear();
        let view = page.borrow().view(&mut PageCx {
            scope,
            cx,
            subscribed: &mut subscribed,
            page: PhantomData,
        });
        crate::slots::drawn(scope);
        crate::devtools::record(scope, &view);
        marked::<P>(view)
    }
}

pub struct LayoutNode<L> {
    /// See `PageNode::page`.
    layout: Option<Shared<L>>,
    scope: Scope,
    /// See `PageNode::subscribed`.
    subscribed: RefCell<Vec<TypeId>>,
}

impl<L: Layout> Component for LayoutNode<L> {
    type Input = Scope;
    type Message = Signal<L::Message>;

    fn create(scope: &Scope, _cx: &ComponentContext<Self>) -> Self {
        #[cfg(feature = "harness")]
        {
            let sender = _cx.sender();
            crate::harness::remember::<L, L::Message>(move |signal| sender.send(signal));
        }

        Self {
            layout: staged::<L>(*scope),
            scope: *scope,
            subscribed: RefCell::default(),
        }
    }

    fn input_changed(&mut self, scope: &Scope, _cx: &ComponentContext<Self>) {
        if *scope != self.scope {
            self.layout = staged::<L>(*scope);
            self.scope = *scope;
        }
    }

    fn update(&mut self, message: Signal<L::Message>, cx: &ComponentContext<Self>) {
        match message {
            Signal::Refresh(cause) => handling(cause),
            Signal::OpenWindow(window) => open(cx, window),
            Signal::Node(message) => {
                let Some(layout) = self.layout.as_ref().filter(|_| !left(self.scope)) else {
                    return;
                };
                layout.borrow_mut().update(
                    message,
                    &mut UpdateCx {
                        scope: self.scope,
                        node: PhantomData,
                    },
                )
            }
        }
    }

    /// Draws the layout in a pass of its own: what it mounts this time is
    /// what stands under it, and what it left out goes when the pass ends.
    fn view(&self, scope: &Scope, cx: &mut ViewContext<Self>) -> View {
        let scope = *scope;
        let Some(layout) = self.layout.as_ref().filter(|_| !left(scope)) else {
            return marked::<L>(Grid::new().into());
        };
        let _caused = caused(cx);
        let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
        leaves_its_fills(scope, cx);
        crate::slots::drawing(scope);

        let pass = Pass::begin(scope);
        let mut subscribed = self.subscribed.borrow_mut();
        subscribed.clear();
        let view = layout.borrow().view(&mut LayoutCx {
            scope,
            pass: &pass,
            cx,
            subscribed: &mut subscribed,
            layout: PhantomData,
        });
        pass.end();

        crate::slots::drawn(scope);
        crate::devtools::record(scope, &view);
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

/// Whether `scope` was removed while a redraw of what it holds, or a message
/// for it, was on its way.
///
/// A push queues the redraw a drain ahead of the drawing; the layout above
/// removes a page it no longer mounts as its own pass ends. The redraw that
/// comes after finds nothing to read - so it draws nothing. A message for
/// the page is dropped the same way: `update` would read a scope that is gone.
fn left(scope: Scope) -> bool {
    !scope.is_alive()
}

/// Withdraws what a node filled, and the slots it placed, when it leaves the
/// screen or is mounted again elsewhere.
fn leaves_its_fills<C: Component>(filler: Scope, cx: &mut ViewContext<C>) {
    cx.use_effect("guinea.fills", filler, move || {
        Some(Box::new(move || crate::slots::left(filler)) as Box<dyn FnOnce()>)
    });
}

/// A page's or a layout's view inside a border whose `AutomationId` is its
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
    Under(Box<dyn FnOnce(&AppContext) -> View>),
}

impl NewWindow {
    /// A window built from the application it is opened from.
    pub fn under(build: impl FnOnce(&AppContext) -> View + 'static) -> Self {
        Self(Opens::Under(Box::new(build)))
    }

    fn open_from(self, scope: Scope) -> View {
        match self.0 {
            Opens::View(view) => view,
            Opens::Under(build) => {
                let app = crate::run::application(scope).unwrap_or_else(|| {
                    panic!(
                        "a window of the application is opened from a view in a window of \
                         one - this view is in none"
                    )
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

/// What redraws a tree the reactor draws: each component that read a reducer
/// that moved is asked to publish again.
pub(crate) struct Reactor;

impl guinea_core::draws::Draws for Reactor {
    type Reader = Refresher;

    fn redraw(&self, readers: guinea_core::draws::Readers<'_, Refresher>) {
        for component in readers {
            component.refresh();
        }
    }
}

/// What a node's `update` is handed.
///
/// Reading and acting, and nothing else. Navigation is not here on purpose: it
/// starts from a widget event, and the handle a view already has is captured
/// into the callback where the event is declared - which reads better than
/// fetching one again in the place the event arrives.
pub struct UpdateCx<'a, S> {
    scope: Scope,
    node: PhantomData<fn() -> &'a S>,
}

impl<S> UpdateCx<'_, S> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// No subscription: `update` is a moment, not a view, and the node is
    /// already publishing again because of the message that got here.
    pub fn read<R>(&self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
    {
        let binding = binding::<R>(self.scope);
        (binding.get(), binding.dispatch())
    }
}

/// Reads a reducer's state, and asks this node to publish again whenever it
/// changes.
///
/// The state is read fresh rather than mirrored into the node. A reducer lives
/// in the scope and changes from under the view, so a copy held here would be
/// one more thing to keep in step; the subscription says *when* to look, and
/// looking is `binding.get()`.
///
/// Taken again when the component is handed another scope: the one before
/// listens to a scope that is gone.
///
/// Taken once per reducer per render: `subscribed` lists what this render
/// already listens to, and a second read of the same reducer shares it.
fn use_reducer<R, C>(
    scope: Scope,
    cx: &mut ViewContext<C>,
    subscribed: &mut Vec<TypeId>,
) -> (Rc<R>, guinea_core::feature::Dispatch)
where
    R: Reducer + PartialEq,
    C: Refreshable,
{
    let binding = binding::<R>(scope);

    if !subscribed.contains(&TypeId::of::<R>()) {
        subscribed.push(TypeId::of::<R>());
        let refresher = Refresher::of(cx);

        // Keyed by the reducer, so a view reading two of them gets two
        // subscriptions rather than one that replaces the other.
        let watching = binding.clone();
        cx.use_effect(std::any::type_name::<R>(), scope, move || {
            let subscription = watching.read_by(refresher);
            // Ends with this component instance rather than with the scope: one
            // that had been unmounted and kept listening would be asking a
            // reconciler slot nobody renders to publish.
            Some(Box::new(move || drop(subscription)) as Box<dyn FnOnce()>)
        });
    }

    (binding.get(), binding.dispatch())
}

/// Watches `nav` for as long as this component stands under `scope`, and
/// redraws it whenever the navigator moves or a guard's question comes or
/// goes.
///
/// Once per navigator per render: `subscribed` lists what this render already
/// watches, beside the reducers it reads.
fn watching<R: Routable, C: Refreshable>(
    nav: &Navigator<R>,
    scope: Scope,
    cx: &mut ViewContext<C>,
    subscribed: &mut Vec<TypeId>,
) {
    if subscribed.contains(&TypeId::of::<Navigator<R>>()) {
        return;
    }
    subscribed.push(TypeId::of::<Navigator<R>>());

    let refresher = Refresher::of(cx);
    let nav = nav.clone();
    cx.use_effect(std::any::type_name::<Navigator<R>>(), scope, move || {
        let watch = nav.watch(move || refresher.refresh());
        Some(Box::new(move || drop(watch)) as Box<dyn FnOnce()>)
    });
}

/// What a page's view is handed.
///
/// Carries the page type, not because rendering needs it, but because reading
/// does: what a page may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, 'v, P: Page> {
    scope: Scope,
    cx: &'a mut ViewContext<'v, PageNode<P>>,
    subscribed: &'a mut Vec<TypeId>,
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
        let window = window.into().open_from(self.scope);
        let sender = self.cx.sender();
        let window = RefCell::new(Some(window));
        Callback::new(move |_| {
            if let Some(window) = window.borrow_mut().take()
                && !sender.send(Signal::OpenWindow(window))
            {
                tracing::warn!("the page asking for a window is gone; not opened");
            }
        })
    }

    /// The navigator over `R` nearest above this page, and a redraw of the
    /// page whenever it moves.
    ///
    /// Panics when none was opened above: a page that navigates where nothing
    /// navigates is a wiring mistake.
    pub fn navigate<R: Routable>(&mut self) -> Navigator<R> {
        let nav = navigator::<R>(self.scope);
        watching(&nav, self.scope, self.cx, self.subscribed);
        nav
    }

    /// Shows `view` in the slot `S` the nearest layout above placed. See
    /// [`LayoutCx::slot`].
    ///
    /// The innermost fill wins. A fill is made again on every draw, and one
    /// not made again is withdrawn, as is everything a page filled when it
    /// leaves. Callbacks in `view` belong to this page, so a toolbar button in
    /// the layout acts on the page.
    ///
    /// Filling a slot no layout above placed panics in a debug build.
    pub fn fill<S: Slot>(&mut self, view: View) {
        crate::slots::fill::<S>(self.scope, view);
    }

    /// The bus of the application this page is in.
    pub fn bus(&self) -> Rc<guinea_core::actor::event_bus::EventBus> {
        self.scope.bus()
    }
}

impl<P: Page> PageCx<'_, '_, P> {
    /// Reads a reducer's state, and asks this page to publish again when it
    /// changes.
    ///
    /// The feature that answers is the one this page installed, or the nearest
    /// above that listed `R` in `Exports`. A read that reaches nothing panics
    /// here, naming what the page was mounted under.
    pub fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(self.scope, self.cx, self.subscribed)
    }
}

impl<P: Page> Reads for PageCx<'_, '_, P> {
    fn __scope(&self) -> Scope {
        self.scope
    }

    fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(self.scope, self.cx, self.subscribed)
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

/// What a layout's view is handed: everything a [`PageCx`] is, and mounting.
pub struct LayoutCx<'a, 'v, L: Layout> {
    scope: Scope,
    pass: &'a Pass,
    cx: &'a mut ViewContext<'v, LayoutNode<L>>,
    subscribed: &'a mut Vec<TypeId>,
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

    /// Mounts `S` - a page or a layout - under this layout, with `params`.
    ///
    /// Installed the first time this layout's view asks for it, in a scope of
    /// its own under the layout's, and the same one every draw after that
    /// while the params are equal. Other params are another child. A child
    /// the view stops asking for goes when the view returns - unless it was
    /// [`keep`](Child::keep)s, which puts it to sleep instead.
    ///
    /// One that fails to install leaves nothing behind, and draws why in its
    /// place.
    pub fn mount<S: Mountable>(&mut self, params: &S::Params) -> Child {
        let mounted = self.pass.mount::<S, _>(params, |cx| S::install(cx, params));

        Child {
            pass: self.pass.clone(),
            mounted,
        }
    }

    /// What `nav` says is here, mounted by `draw`: the layout's `match` on
    /// its own level of the route.
    ///
    /// The layout redraws whenever `nav` moves, and `draw` is asked again.
    /// What it mounted is what the navigator asks before it moves on - its
    /// leave guards, and everything under it. A child that fails to mount
    /// puts the navigator back where it was, keeps what stood here before,
    /// and leaves why on the navigator's
    /// [`failure`](guinea_router::navigator::Navigator::failure).
    pub fn outlet<R: Routable>(
        &mut self,
        nav: &Navigator<R>,
        draw: impl FnOnce(&R, &mut Self) -> Child,
    ) -> View {
        watching(nav, self.scope, self.cx, self.subscribed);

        let pass = self.pass.clone();
        match pass.outlet(nav, |here| draw(here, self).mounted) {
            Outlet::Here(scope) | Outlet::Held(scope) => drawn(scope),
            Outlet::Failed(error) => failed(&error),
            Outlet::Nowhere => Grid::new().into(),
        }
    }

    /// See [`PageCx::open_window`].
    pub fn open_window(&mut self, window: impl Into<NewWindow>) -> Callback<()> {
        let window = window.into().open_from(self.scope);
        let sender = self.cx.sender();
        let window = RefCell::new(Some(window));
        Callback::new(move |_| {
            if let Some(window) = window.borrow_mut().take()
                && !sender.send(Signal::OpenWindow(window))
            {
                tracing::warn!("the layout asking for a window is gone; not opened");
            }
        })
    }

    /// See [`PageCx::navigate`].
    pub fn navigate<R: Routable>(&mut self) -> Navigator<R> {
        let nav = navigator::<R>(self.scope);
        watching(&nav, self.scope, self.cx, self.subscribed);
        nav
    }

    /// Places the slot `S`: what the innermost page or layout below filled it
    /// with, or nothing.
    ///
    /// Placing it is what declares it. The fill arrives a drain after the
    /// node that made it drew, and redraws the slot alone - not this layout,
    /// and not what it mounted.
    pub fn slot<S: Slot>(&mut self) -> View {
        crate::slots::place::<S>(self.scope, None)
    }

    /// [`slot`](Self::slot), showing `standing` while nothing fills it.
    pub fn slot_or<S: Slot>(&mut self, standing: impl Into<View>) -> View {
        crate::slots::place::<S>(self.scope, Some(standing.into()))
    }

    /// See [`PageCx::fill`]. A layout fills a slot a layout above placed.
    pub fn fill<S: Slot>(&mut self, view: View) {
        crate::slots::fill::<S>(self.scope, view);
    }

    /// The bus of the application this layout is in.
    pub fn bus(&self) -> Rc<guinea_core::actor::event_bus::EventBus> {
        self.scope.bus()
    }
}

impl<L: Layout> LayoutCx<'_, '_, L> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(self.scope, self.cx, self.subscribed)
    }
}

impl<L: Layout> Reads for LayoutCx<'_, '_, L> {
    fn __scope(&self) -> Scope {
        self.scope
    }

    fn read<R>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        use_reducer::<R, _>(self.scope, self.cx, self.subscribed)
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
