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

use guinea_app::feature::{FeatureInitContext, Reaches, Segment};
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, RouteSink, Router, SegmentEntry, SegmentProps, Ui,
    single_entry_chain,
};
use windows_reactor::{
    AutomationExt, Border, Callback, Component, ComponentContext, ContentControl, View, ViewContext,
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
///   with [`use_reducer`](PageCx::use_reducer), turns a widget's event into a
///   message with [`on`](PageCx::on), and asks a feature for something with
///   `emit` on the dispatch it was handed.
///
/// `#[page]` writes what a page leaves out: `Params = ()` for a page that
/// captures nothing, `Installs = ()` with its empty `install`, and a
/// `Message` nobody can send with its empty `update`. The smallest page is a
/// view:
///
/// ```
/// use guinea_winui::{Page, PageCx, page};
/// use windows_reactor::{TextBlock, View};
///
/// #[derive(Default)]
/// struct About;
///
/// #[page]
/// impl Page for About {
///     fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
///         TextBlock::new().text("guinea").into()
///     }
/// }
/// ```
///
/// A whole one. The feature keeps a count and adds to it when asked; the page
/// shows the count, keeps a step of its own, and asks for the step to be
/// added:
///
/// ```
/// # use guinea_app::feature::Segment;
/// # use guinea_core::scope::Reducer;
/// # use guinea_macros::{feature, installs};
/// use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
/// use windows_reactor::{Button, ChildrenControl, ContentControl, StackPanel, TextBlock, View};
///
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Count(pub u32);
///
/// impl Reducer for Count {
///     type Update = u32;
///
///     fn reduce(&mut self, by: u32) {
///         self.0 += by;
///     }
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
///     fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
///         let (count, dispatch) = cx.use_reducer::<Count, _>();
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
/// // `routes!` writes where each segment sits; this page stands alone.
/// impl Segment for CounterPage {
///     type Installs = Counter;
///     type Above = ();
/// }
///
/// // Under test it mounts with no window, in a harness that runs what it
/// // sets off in a seeded order - `harness::Mounted`, behind the `harness`
/// // feature. `#[guinea::test]` is this `check` as an attribute.
/// #[cfg(feature = "harness")]
/// guinea_app::app::check(4, |h| {
///     let mut page = guinea_winui::harness::Mounted::<CounterPage>::mount(&h.segment(), ())
///         .unwrap();
///
///     page.click_text("Bigger").settle();
///     page.click_text("Add").settle();
///     page.settle();
///
///     assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
/// });
/// ```
pub trait Page: Default + Sized + 'static {
    /// When `true`, the router keeps this page's reducer states in memory
    /// while the page is not mounted. The page's scope (and therefore its
    /// actors) is still torn down, but when the user returns the UI will see
    /// the last cached state immediately instead of starting from defaults.
    const CACHE_STATE_IN_MEMORY: bool = false;

    /// Where `impl Page` was written. `#[segment]` fills it in; an impl
    /// without it loses only the source link.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What this page captured from the route, named by `routes!`. `()` for a
    /// page that captures nothing.
    ///
    /// `PartialEq` because the router's one question about a capture is
    /// whether it is still the same one - which decides what reinstalls and
    /// which cached state may come back.
    type Params: PartialEq + 'static;

    /// What this segment installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking.
    ///
    /// What is returned is owned by the segment's scope, which is what gives a
    /// feature its own lifetime.
    type Installs: 'static;

    /// This page's own events. An enum, and nobody else's business.
    type Message: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// The node it starts as, when `Default` is not it.
    ///
    /// A constructor, not an effect: it runs on every install, and anything
    /// that must happen exactly once per mount belongs in
    /// [`install`](Self::install).
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Asked before this page is left, and answered from its own state.
    ///
    /// The asymmetry with entering is the whole reason it is a method here: on
    /// the way out the node exists, so it can say whether it minds - which is
    /// what "unsaved changes" is. On the way in there is nothing to ask.
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    /// The only place the node changes.
    ///
    /// Effects are actions emitted to features - `cx.state::<R>().1.emit(..)` -
    /// rather than values returned from here: an effect that crosses a segment
    /// boundary is a domain's job, and one that does not is a state change.
    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View;
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
/// ```
/// # use guinea_app::feature::Segment;
/// # use guinea_core::feature::Bound;
/// # use guinea_core::scope::Reducer;
/// # use guinea_macros::{feature, installs};
/// use guinea_winui::{
///     FeatureInitContext, Layout, LayoutCx, Page, PageCx, UpdateCx, layout, page,
/// };
/// use windows_reactor::{Button, ChildrenControl, ContentControl, StackPanel, TextBlock, View};
///
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Sidebar {
///     pub open: bool,
/// }
///
/// impl Reducer for Sidebar {
///     type Update = bool;
///
///     fn reduce(&mut self, open: bool) {
///         self.open = open;
///     }
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
/// impl Reducer for Title {
///     type Update = String;
///
///     fn reduce(&mut self, title: String) {
///         self.0 = title;
///     }
/// }
///
/// #[derive(Default)]
/// pub struct Home;
///
/// #[page]
/// impl Page for Home {
///     fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
///         // Installed by the shell above, and readable here because `Chrome`
///         // exports it. A page outside the shell asking for it does not
///         // compile.
///         let (sidebar, _) = cx.use_reducer::<Sidebar, _>();
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
///                 let (sidebar, dispatch) = cx.state::<Sidebar, _>();
///                 dispatch.emit(SetOpen(!sidebar.open));
///             }
///         }
///     }
///
///     fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
///         let (sidebar, _) = cx.use_reducer::<Sidebar, _>();
///         let (title, _) = cx.use_reducer::<Title, _>();
///
///         let tab = if cx.child_is::<Home>() { "> Home" } else { "Home" };
///         let side: View = if sidebar.open {
///             TextBlock::new().text(tab).into()
///         } else {
///             View::empty()
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
///
/// // Where each segment sits, which is what `routes!` writes.
/// impl Segment for Shell {
///     type Installs = <Shell as Layout>::Installs;
///     type Above = ();
/// }
///
/// impl Segment for Home {
///     type Installs = ();
///     type Above = (Shell, ());
/// }
/// ```
///
/// Nothing reaches up past what is above it. A page that no layout above
/// installed `Sidebar` for does not compile at the read:
///
/// ```compile_fail,E0277
/// # use guinea_app::feature::Segment;
/// # use guinea_core::scope::Reducer;
/// use guinea_winui::{Page, PageCx, page};
/// use windows_reactor::{TextBlock, View};
///
/// #[derive(Default, Clone, PartialEq, Debug)]
/// pub struct Sidebar {
///     pub open: bool,
/// }
///
/// impl Reducer for Sidebar {
///     type Update = bool;
///
///     fn reduce(&mut self, open: bool) {
///         self.open = open;
///     }
/// }
///
/// #[derive(Default)]
/// pub struct Settings;
///
/// #[page]
/// impl Page for Settings {
///     fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
///         let (sidebar, _) = cx.use_reducer::<Sidebar, _>();
///         TextBlock::new().text(format!("{}", sidebar.open)).into()
///     }
/// }
///
/// impl Segment for Settings {
///     type Installs = ();
///     type Above = ();
/// }
/// ```
pub trait Layout: Default + Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What every page under this layout carries, derived by `routes!` as the
    /// intersection of their parameters. A layout declares nothing; it is
    /// handed what all of its children were reached with.
    type Params: PartialEq + 'static;

    /// What this segment installs. See [`Page::Installs`].
    type Installs: 'static;

    type Message: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Asked before this layout is left. See [`Page::leaving`].
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View;
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<WinUi> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        guinea_router::router::same_params::<P::Params>,
        &const { MountPage::<P>(PhantomData) },
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<WinUi> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        guinea_router::router::same_params::<L::Params>,
        &const { MountLayout::<L>(PhantomData) },
        false,
    )
    .written(L::DECLARED)
}

thread_local! {
    /// Nodes built by `install`, waiting for the reconciler to mount them.
    ///
    /// `init` needs the feature context, which exists while the router is
    /// installing; a component is created later, when the reconciler reaches
    /// it, and is handed no such thing. So the node is built where the context
    /// is and taken where it is needed. Keyed by type: a chain holds each
    /// segment type once.
    static STAGED: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

fn stage<S: 'static>(node: S) {
    STAGED.with(|staged| {
        staged.borrow_mut().insert(TypeId::of::<S>(), Box::new(node));
    });
}

/// The staged node, or a default one - which is what a page mounted outside a
/// route tree gets, and what a second publication of the same segment gets.
fn take_staged<S: Default + 'static>() -> S {
    STAGED
        .with(|staged| staged.borrow_mut().remove(&TypeId::of::<S>()))
        .and_then(|node| node.downcast::<S>().ok())
        .map(|node| *node)
        .unwrap_or_default()
}

pub(crate) fn install_page<P: Page>(ctx: &FeatureInitContext, params: &dyn Any) -> anyhow::Result<()> {
    let params = guinea_router::router::narrow::<P::Params, P>(params)?;
    own(ctx, P::install(ctx, params)?);
    stage(P::init(ctx, params));
    Ok(())
}

/// Hands what a segment installed to its scope - a feature's lifetime is the
/// segment's, and dropping this here would end it at the end of `install`.
fn own<T: 'static>(ctx: &FeatureInitContext, installed: T) {
    ctx.scope.own(guinea_core::scope::DropGuard(installed));
}

fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &dyn Any) -> anyhow::Result<()> {
    let params = guinea_router::router::narrow::<L::Params, L>(params)?;
    own(ctx, L::install(ctx, params)?);
    stage(L::init(ctx, params));
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
    /// It carries nothing, because the new state is read from the reducer
    /// rather than delivered. The message only says *when*.
    Refresh,
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
        Signal::Refresh
    }
}

impl<L: Layout> Refreshable for LayoutNode<L> {
    fn refresh() -> Self::Message {
        Signal::Refresh
    }
}

/// A page as the reconciler sees it: the page's own state, and nothing else.
///
/// The component's `Input` is the segment's props, which the reconciler already
/// compares for us - `SegmentProps` has the `PartialEq` that decides whether
/// this subtree is still the same one.
pub struct PageNode<P> {
    page: P,
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
            crate::harness::remember::<P>(move |signal| sender.send(signal));
        }

        Self {
            page: take_staged::<P>(),
            props: input.clone(),
        }
    }

    fn input_changed(&mut self, input: &Self::Input, _cx: &ComponentContext<Self>) {
        self.props = input.clone();
    }

    fn update(&mut self, message: Signal<P::Message>, cx: &ComponentContext<Self>) {
        match message {
            // Nothing to change. Delivering a message is itself what marks the
            // segment for redrawing - the pump pushes the token onto its dirty
            // list when it dispatches, not when something is mutated - so a
            // refresh that touches no state still brings the view round.
            Signal::Refresh => {}
            Signal::OpenWindow(window) => open(cx, window),
            Signal::Node(message) => self.page.update(
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
            guinea_core::devtools::profiling::frame_done();
        }
        let _drawing = guinea_core::devtools::Rendering::of(std::any::type_name::<P>());
        let view = self.page.view(&mut PageCx {
            props: input.clone(),
            cx,
            page: PhantomData,
        });
        crate::devtools::record(input, &view);
        marked::<P>(view)
    }
}

pub struct LayoutNode<L> {
    layout: L,
    props: SegmentProps<WinUi>,
}

impl<L: Layout> Component for LayoutNode<L> {
    type Input = SegmentProps<WinUi>;
    type Message = Signal<L::Message>;

    fn create(input: &Self::Input, _cx: &ComponentContext<Self>) -> Self {
        Self {
            layout: take_staged::<L>(),
            props: input.clone(),
        }
    }

    fn input_changed(&mut self, input: &Self::Input, _cx: &ComponentContext<Self>) {
        self.props = input.clone();
    }

    fn update(&mut self, message: Signal<L::Message>, cx: &ComponentContext<Self>) {
        match message {
            // See `PageNode::update`.
            Signal::Refresh => {}
            Signal::OpenWindow(window) => open(cx, window),
            Signal::Node(message) => self.layout.update(
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
            guinea_core::devtools::profiling::frame_done();
        }
        let _drawing = guinea_core::devtools::Rendering::of(std::any::type_name::<L>());
        let view = self.layout.view(&mut LayoutCx {
            props: input.clone(),
            cx,
            layout: PhantomData,
        });
        crate::devtools::record(input, &view);
        marked::<L>(view)
    }
}

/// A segment's view inside a border whose `AutomationId` is the segment's
/// name, so the XAML tree shows where each page and layout begins.
fn marked<S>(view: View) -> View {
    Border::new()
        .automation_id(guinea_router::devtools::short(std::any::type_name::<S>()))
        .content(view)
        .into()
}

fn open<C: Component>(cx: &ComponentContext<C>, window: View) {
    if !cx.open_window(window) {
        tracing::warn!("no active publication; the window was not opened");
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
            sender.send(C::refresh());
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

impl<S: Segment> UpdateCx<'_, S> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// No subscription: `update` is a moment, not a view, and the segment is
    /// already publishing again because of the message that got here.
    pub fn state<R, I>(&self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
        S: Reaches<R, I>,
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
    cx.use_effect(std::any::type_name::<R>(), (), move || {
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

fn nav_context<R: 'static>() -> &'static windows_reactor::Context<Option<NavigateHandle<WinUi, R>>>
{
    context_for::<Option<NavigateHandle<WinUi, R>>>(|| None)
}

fn route_context<R: 'static>() -> &'static windows_reactor::Context<Option<R>> {
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
    _panel: guinea_core::devtools::PanelGuard,
}

impl<R> Component for RouterRoot<R>
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    /// Where the window starts. A second window opened with a different route
    /// is a different input, and gets its own router.
    type Input = R;
    /// Where it went. `NavigateHandle` publishes through the sink below, and
    /// the sink sends this.
    type Message = R;

    fn create(initial: &R, _cx: &ComponentContext<Self>) -> Self {
        let token = guinea_core::actor::UiThreadToken::dangerously_create_token_unchecked();
        let router = Rc::new(Router::new(token));
        if guinea_app::app::roots::labelled(crate::run::MAIN).is_none() {
            guinea_app::app::roots::set_label(router.root(), crate::run::MAIN);
        }

        // Before the first view, so the tree exists by the time anything asks
        // to render it. A failure here is fatal to the window, and `create`
        // cannot report one.
        router
            .navigate(initial.clone())
            .expect("the initial route installs");

        Self {
            _panel: crate::devtools::offer(&router),
            router,
            route: initial.clone(),
        }
    }

    fn update(&mut self, route: R, _cx: &ComponentContext<Self>) {
        if let Err(error) = self.router.navigate(route.clone()) {
            tracing::error!(%error, "navigation failed");
            return;
        }
        self.route = route;
    }

    fn view(&self, _input: &R, cx: &mut ViewContext<Self>) -> View {
        let sender = cx.sender();
        let nav = NavigateHandle::new(
            self.router.clone(),
            RouteSink::new(move |route: R| {
                sender.send(route);
            }),
        );

        let tree = self.router.render(&());
        let tree = View::provide(
            router_context(),
            Some(RouterHandle(self.router.clone())),
            tree,
        );
        let tree = View::provide(route_context::<R>(), Some(self.route.clone()), tree);
        View::provide(nav_context::<R>(), Some(nav), tree)
    }
}

/// Subscribing to route changes from a view, the way any other effect is
/// written: registered once, undone when the view unmounts.
pub trait UseRouteChange {
    /// Runs `hook` after each navigation, for as long as this view is mounted.
    /// Panics if there is no router above - a wiring mistake, not a state a
    /// running application can reach.
    fn use_route_change(&mut self, hook: impl Fn(Option<&str>, &str) + 'static);
}

impl<C: Component> UseRouteChange for ViewContext<C> {
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

impl<C: Component> UseNavigate for ViewContext<C> {
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

impl<C: Component> UseRoute for ViewContext<C> {
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
pub struct PageCx<'a, P: Page> {
    props: SegmentProps<WinUi>,
    cx: &'a mut ViewContext<PageNode<P>>,
    page: PhantomData<fn() -> P>,
}

impl<P: Page> PageCx<'_, P> {
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
    pub fn open_window(&self, window: View) -> Callback<()> {
        let sender = self.cx.sender();
        let window = RefCell::new(Some(window));
        Callback::new(move |_| {
            if let Some(window) = window.borrow_mut().take() {
                sender.send(Signal::OpenWindow(window));
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
}

impl<P: Page + Segment> PageCx<'_, P> {
    /// Reads a reducer's state, and asks this segment to publish again when it
    /// changes.
    ///
    /// Which feature answers is settled at build time: this page installed it,
    /// or a segment above listed it in `Exports`. The `_` is [`Reaches`]'s
    /// index, which says which of several impls applied - Rust has no partial
    /// turbofish, so it has to be written.
    pub fn use_reducer<R, I>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
        P: Reaches<R, I>,
    {
        use_reducer::<R, _>(&self.props, self.cx)
    }
}

impl<P: Page> std::ops::Deref for PageCx<'_, P> {
    type Target = ViewContext<PageNode<P>>;
    fn deref(&self) -> &Self::Target {
        self.cx
    }
}

impl<P: Page> std::ops::DerefMut for PageCx<'_, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}

pub struct LayoutCx<'a, L: Layout> {
    props: SegmentProps<WinUi>,
    cx: &'a mut ViewContext<LayoutNode<L>>,
    layout: PhantomData<fn() -> L>,
}

impl<L: Layout> LayoutCx<'_, L> {
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
    pub fn open_window(&self, window: View) -> Callback<()> {
        let sender = self.cx.sender();
        let window = RefCell::new(Some(window));
        Callback::new(move |_| {
            if let Some(window) = window.borrow_mut().take() {
                sender.send(Signal::OpenWindow(window));
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
}

impl<L: Layout + Segment> LayoutCx<'_, L> {
    /// See [`PageCx::use_reducer`].
    pub fn use_reducer<R, I>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
        L: Reaches<R, I>,
    {
        use_reducer::<R, _>(&self.props, self.cx)
    }
}

impl<L: Layout> std::ops::Deref for LayoutCx<'_, L> {
    type Target = ViewContext<LayoutNode<L>>;
    fn deref(&self) -> &Self::Target {
        self.cx
    }
}

impl<L: Layout> std::ops::DerefMut for LayoutCx<'_, L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}
