//! guinea on egui: the same router and features, drawn immediately.
//!
//! The closest relative among the backends is ratatui, not WinUI: egui is
//! immediate, so a view is not a tree that is kept and diffed - it is drawing
//! that happens inside one frame and leaves nothing behind. As there, a view
//! is a [`Node`]: drawing deferred until someone supplies the [`egui::Ui`] to
//! draw into, which is what lets a layout decide where its child goes.
//!
//! What differs from the terminal is who owns the loop. eframe owns it, the
//! way the reactor does under WinUI, so [`run`] hands it over and puts the
//! frame inside `eframe::App::update`. And unlike a terminal, egui sleeps when
//! nothing happens - so work finished on another thread has to wake it, which
//! is what the dispatcher's `request_repaint` is for.

mod dispatcher;
mod nav;
mod run;

pub use run::{MAIN, run};

use guinea_app::feature::{FeatureInitContext, Reads};
use guinea_core::binding::ReducerBinding;
use guinea_core::scope::Reducer;
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, SegmentEntry, SegmentProps, Ui, single_entry_chain,
};

/// egui as a [`Ui`].
pub struct Egui;

impl Ui for Egui {
    type View<'a> = Node;
    /// Nothing: an immediate-mode view draws from a snapshot inside the frame
    /// and holds no reference to state afterwards.
    type Nodes = ();
    type Mount = dyn Mount<Self>;
}

/// Drawing that has not happened yet.
///
/// `FnOnce` because a node is drawn exactly once per frame - the next frame
/// mounts fresh ones.
pub struct Node(Box<dyn FnOnce(&mut egui::Ui)>);

impl Node {
    pub fn new(draw: impl FnOnce(&mut egui::Ui) + 'static) -> Self {
        Self(Box::new(draw))
    }

    /// Draws into `ui`.
    pub fn draw(self, ui: &mut egui::Ui) {
        (self.0)(ui)
    }
}

/// A leaf of the route tree, and its own state.
///
/// The struct that implements this **is** the page's state, as in the WinUI
/// and iced backends. What differs is that immediate mode has no later: the
/// frame that sees the click is the frame that answers it, so there is no
/// message and no `update` - [`Page::render`] takes `&mut self` and writes
/// what it decided where it decided it.
///
/// What belongs here is what only this page has an opinion about: which row
/// is picked, which tab is open, what is typed in a filter. What crosses the
/// segment - what a domain owns, what another page reads - is a reducer, and
/// reaches this page through [`PageCx::state`].
pub trait Page: Default + Sized + 'static {
    /// When `true`, the router keeps this page's reducer states in memory
    /// while the page is not mounted.
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
    /// installed stops type-checking. Which is also why `install` has no
    /// default any more.
    ///
    /// What is returned is owned by the segment's scope, which is what gives a
    /// feature its own lifetime.
    type Installs: guinea_app::feature::Lists;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// The state it starts with, when `Default` is not it.
    ///
    /// A constructor, not an effect: it runs once per mount, beside
    /// [`install`](Self::install), and anything that has to reach a feature
    /// belongs there instead.
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Draws the page, and changes it. Runs again for every frame, so this is
    /// the drawing itself and not a description of it.
    fn render(&mut self, cx: &mut PageCx<'_, Self>);
}

/// A branch: draws its own chrome and decides where its child goes. Its own
/// state, the same way a [`Page`] is.
pub trait Layout: Default + Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What every page under this layout carries, derived by `routes!` as the
    /// intersection of their parameters. A layout declares nothing; it is
    /// handed what all of its children were reached with.
    type Params: PartialEq + 'static;

    /// What this segment installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking. Which is also why `install` has no
    /// default any more.
    ///
    /// What is returned is owned by the segment's scope, which is what gives a
    /// feature its own lifetime.
    type Installs: guinea_app::feature::Lists;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// See [`Page::init`].
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    fn render(&mut self, cx: &mut LayoutCx<'_, Self>);
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<Egui> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        guinea_router::router::same_params::<P::Params>,
        <P::Installs as guinea_app::feature::Lists>::list,
        &const { MountPage::<P>(std::marker::PhantomData) } as &dyn Mount<Egui>,
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<Egui> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        guinea_router::router::same_params::<L::Params>,
        <L::Installs as guinea_app::feature::Lists>::list,
        &const { MountLayout::<L>(std::marker::PhantomData) } as &dyn Mount<Egui>,
        false,
    )
    .written(L::DECLARED)
}

fn install_page<P: Page>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    let params = guinea_router::router::narrow::<P::Params, P>(params)?;
    own(ctx, P::install(ctx, params)?);
    guinea_router::mounted::keep(&ctx.scope, P::init(ctx, params));
    Ok(())
}

/// Hands what a segment installed to its scope - a feature's lifetime is the
/// segment's, and dropping this here would end it at the end of `install`.
fn own<T: 'static>(ctx: &FeatureInitContext, installed: T) {
    ctx.scope.own(guinea_core::scope::DropGuard(installed));
}

fn install_layout<L: Layout>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    let params = guinea_router::router::narrow::<L::Params, L>(params)?;
    own(ctx, L::install(ctx, params)?);
    guinea_router::mounted::keep(&ctx.scope, L::init(ctx, params));
    Ok(())
}

/// A zero-sized marker per segment type: what a `const` entry points at to get
/// its `&'static dyn Mount`.
pub struct MountPage<P>(pub std::marker::PhantomData<P>);
pub struct MountLayout<L>(pub std::marker::PhantomData<L>);

impl<P: Page> Mount<Egui> for MountPage<P> {
    fn view<'a>(&self, props: SegmentProps<Egui>, _nodes: &'a ()) -> Node {
        let at = props.scopes[props.cursor];

        Node::new(move |ui| {
            let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
            guinea_router::mounted::with::<P, _>(&at, |page| {
                page.render(&mut PageCx {
                    ui,
                    props,
                    page: std::marker::PhantomData,
                })
            })
        })
    }
}

impl<L: Layout> Mount<Egui> for MountLayout<L> {
    fn view<'a>(&self, props: SegmentProps<Egui>, _nodes: &'a ()) -> Node {
        let at = props.scopes[props.cursor];

        Node::new(move |ui| {
            let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
            guinea_router::mounted::with::<L, _>(&at, |layout| {
                layout.render(&mut LayoutCx {
                    ui,
                    props,
                    layout: std::marker::PhantomData,
                })
            })
        })
    }
}

/// A one-segment chain, for a page drawn without a route tree.
pub fn page_chain<P: Page>() -> &'static [SegmentEntry<Egui>] {
    single_entry_chain(segment_entry::<P>())
}

/// What a page's drawing is handed.
///
/// Carries the page type, not because drawing needs it, but because reading
/// does: what a segment may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, P> {
    ui: &'a mut egui::Ui,
    props: SegmentProps<Egui>,
    page: std::marker::PhantomData<fn() -> P>,
}

fn feature_of<R: Reducer>(
    props: &SegmentProps<Egui>,
) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch) {
    let binding = props.binding::<R>();
    (binding.get(), binding.dispatch())
}

impl<P> PageCx<'_, P> {
    /// The reducer's state and actions.
    ///
    /// No subscription, as in the terminal: egui redraws the whole frame, so
    /// there is nothing to invalidate - the next pass reads the state again.
    /// What does need saying is when a frame should happen at all, and that is
    /// the dispatcher's job.
    ///
    /// The feature that answers is the one this page installed, or the nearest
    /// above that listed `R` in `Exports`. A read that reaches nothing panics
    /// here, naming the chain it walked.
    ///
    /// The state comes shared, not copied: reading it every frame costs a
    /// count, and a change made mid-frame goes to a copy.
    pub fn read<R>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(&self.props)
    }

    /// The reducer's binding: its state, and a push straight into it.
    ///
    /// For state the UI owns outright - what is picked in a tree, which tab
    /// is open - claimed with `cx.state::<R>().plain()`. There is no domain
    /// to ask, so there is no actor to ask it through: a click is the whole
    /// story.
    ///
    /// State a feature drives is not this: pushing into it goes behind the
    /// back of whatever answers for it. Use [`PageCx::read`] and emit.
    pub fn binding<R>(&self) -> ReducerBinding<R>
    where
        R: Reducer,
    {
        self.props.binding::<R>()
    }
}

impl<P> Reads for PageCx<'_, P> {
    fn read<R>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        feature_of::<R>(&self.props)
    }

    fn dispatch<R>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
    {
        self.props.binding::<R>().dispatch()
    }
}

impl<P> PageCx<'_, P> {
    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    /// A navigator over the route type the application runs.
    pub fn navigate<R>(&self) -> NavigateHandle<Egui, R>
    where
        R: RouteChain<Egui> + Clone + PartialEq + 'static,
    {
        nav::current::<R>()
    }
}

/// What a layout's drawing is handed. Same as a page's, plus the child.
pub struct LayoutCx<'a, L> {
    ui: &'a mut egui::Ui,
    props: SegmentProps<Egui>,
    layout: std::marker::PhantomData<fn() -> L>,
}

impl<L> LayoutCx<'_, L> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(&self.props)
    }

    /// See [`PageCx::binding`].
    pub fn binding<R>(&self) -> ReducerBinding<R>
    where
        R: Reducer,
    {
        self.props.binding::<R>()
    }
}

impl<L> Reads for LayoutCx<'_, L> {
    fn read<R>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
    {
        feature_of::<R>(&self.props)
    }

    fn dispatch<R>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
    {
        self.props.binding::<R>().dispatch()
    }
}

impl<L> LayoutCx<'_, L> {
    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    /// A navigator over the route type the application runs.
    pub fn navigate<R>(&self) -> NavigateHandle<Egui, R>
    where
        R: RouteChain<Egui> + Clone + PartialEq + 'static,
    {
        nav::current::<R>()
    }

    /// The next segment down the chain, for the layout to draw where it wants.
    ///
    /// Handed over rather than drawn here: a layout takes its `ui` from this
    /// same context, so a method that drew the child would need the context
    /// twice at once.
    pub fn outlet(&self) -> Node {
        self.props.outlet(&())
    }

    /// Whether the segment directly below is `P`.
    ///
    /// What a tab strip needs, and cheaper than it looks: the chain already
    /// says which page is mounted, so highlighting the current tab needs
    /// neither the route nor a copy of it in state.
    pub fn child_is<P: 'static>(&self) -> bool {
        self.props
            .chain
            .get(self.props.cursor + 1)
            .is_some_and(|entry| (entry.type_id)() == std::any::TypeId::of::<P>())
    }
}
