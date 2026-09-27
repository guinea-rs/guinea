//! guinea on iced: Elm inside a node, actors between nodes.
//!
//! The other four backends are told what to draw and have no opinion about
//! where a click goes. iced does: it is Elm, so every interaction is a value
//! of one `Message` type that comes back to one `update`. That is the whole
//! reason this adapter exists - it is the one that has to say what happens
//! when a route tree of independent nodes meets a toolkit that wants a single
//! message type at the top.
//!
//! The answer is that a node keeps its own `State`, its own `Message` and its
//! own `update`, all on the one trait that puts it in the route tree, and
//! nothing above it ever names any of them. A parent does not wrap a child's
//! messages, so adding a page costs no edit anywhere else - the composition
//! tax Elm normally charges (a variant, a match arm, a `map`, per child, per
//! level) is not paid at all. What the toolkit sees is [`Envelope`]: an opaque
//! carrier holding the message, the node's own delivery function, and where in
//! the chain that node sits.
//!
//! What is left over is the part Elm has no answer for: two pieces of state
//! that reference each other rather than nest - a selected row and a list an
//! actor replaces. That is `cx.on`, and it is a translation, not a mutation:
//! it turns what happened elsewhere into this node's own message, so there is
//! still exactly one place the node's state changes.
//!
//! ```ignore
//! #[page]
//! impl Page for Processes {
//!     type Params = ProcessesParams;
//!     type Message = Msg;
//!
//!     fn observes(cx: &Observing<'_, Msg>) { cx.on::<ProcessesReducer>(list_replaced); }
//!     fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) { .. }
//!     fn view(&self, cx: &PageCx<'_, Self>) -> Element<'_, Msg> { .. }
//! }
//! ```

mod dispatcher;
mod envelope;
mod nav;
mod nodes;
mod run;

pub use envelope::Envelope;
pub use nodes::Nodes;
pub use run::{MAIN, run};

/// What a node answers when asked whether it may be left.
pub use guinea_core::guard::{Ask, Verdict};
/// What `install` is handed. Re-exported because `#[page]` writes signatures
/// that name it, and a node should not have to import it just for that.
pub use guinea_app::feature::FeatureInitContext;
/// Writes down the associated types a node left out. See [`Page`].
pub use guinea_macros::{iced_layout as layout, iced_page as page};

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::feature::{Reaches, Segment};
use guinea_core::feature::Dispatch;
use guinea_core::scope::Reducer;
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, SegmentEntry, SegmentProps, Ui, narrow, same_params,
    single_entry_chain,
};

use envelope::Deliver;
use nodes::{Held, Placement};

/// iced as a [`Ui`].
pub struct Iced;

impl Ui for Iced {
    type View<'a> = iced::Element<'a, Envelope>;
    type Nodes = Nodes;
}

/// What a node's `view` returns.
///
/// iced's own name, kept. An alias exists only to fix the message type at the
/// seam; calling it anything else would mean a reader of this adapter has to
/// translate back to iced's vocabulary before reading iced's documentation.
///
/// Borrowed from the node, not owned: iced builds widgets that keep a
/// reference to what they show - `text_editor` holds its `Content` for the
/// life of the element - and a view that had to own everything could not host
/// them. See [`Nodes`] for where the borrow comes from and why.
pub type Element<'a, Message> = iced::Element<'a, Message>;

/// A leaf of the route tree, and an Elm node in its own right.
///
/// The type that implements this **is** the node's state. There is no
/// `type State`, because the struct is already there and already named:
///
/// ```ignore
/// #[derive(Default)]
/// pub struct Processes { row: usize }
/// ```
///
/// That is what an iced application writes anyway - `struct Counter { value:
/// i64 }` - and it removes the last trace of "a view, with its state installed
/// beside it". Where the page sits, what it captured, what it keeps and what
/// can happen to it are one declaration.
pub trait Page: Default + Sized + 'static {
    /// When `true`, this segment's states are kept in memory while the page is
    /// not mounted - the reducers the router owns, and the node itself.
    const CACHE_STATE_IN_MEMORY: bool = false;

    /// Where `impl Page` was written. `#[segment]` fills it in; an impl
    /// without it loses only the source link.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What this page captured from the route, named by `routes!`.
    ///
    /// `PartialEq` because the router's one question about a capture is
    /// whether it is still the same one; `Clone` because the answer is asked
    /// again later, against what a kept node was mounted with.
    type Params: Clone + PartialEq + 'static;

    /// What can happen to this node. `Send` because iced's message type is,
    /// and this one travels inside it.
    type Message: Send + 'static;

    /// What this segment installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking. `#[page]` fills in both this and the
    /// empty `install` when a segment installs nothing.
    ///
    /// What is returned is owned by the segment's scope, which is what gives a
    /// feature its own lifetime.
    type Installs: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// The node it starts as, when `Default` is not it.
    ///
    /// A constructor, not an effect: it runs on every install, and its result
    /// is discarded when a kept node comes back instead. Anything that must
    /// happen exactly once per mount belongs in [`install`](Self::install).
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Which reducers this page translates, and into what.
    fn observes(_cx: &Observing<'_, Self::Message>) {}

    /// Asked before this page is left, and answered from its own state.
    ///
    /// The asymmetry with entering is the whole reason it is a method here:
    /// on the way out the node exists, so it can say whether it minds - which
    /// is what "unsaved changes" is. On the way in there is nothing to ask.
    ///
    /// [`Verdict::Allow`] allocates nothing, so a page that never minds costs
    /// nothing for having the option.
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    /// The only place the node changes.
    ///
    /// Effects are messages to actors - `cx.state::<R>().1.emit(..)` - not
    /// values returned from here. That is the trade this framework makes
    /// against `Task`: an effect that crosses a node boundary is an actor's
    /// job, and one that does not is a state change.
    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    fn view(&self, cx: &PageCx<'_, Self>) -> Element<'_, Self::Message>;
}

/// A branch: an Elm node that also decides where its child goes.
pub trait Layout: Default + Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What every page under this layout carries, derived by `routes!` as the
    /// intersection of their parameters. A layout declares nothing; it is
    /// handed what all of its children were reached with, which is how a tab
    /// strip navigates to a sibling without inventing the context itself.
    type Params: Clone + PartialEq + 'static;

    type Message: Send + 'static;

    /// What this segment installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking. `#[page]` fills in both this and the
    /// empty `install` when a segment installs nothing.
    ///
    /// What is returned is owned by the segment's scope, which is what gives a
    /// feature its own lifetime.
    type Installs: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    fn observes(_cx: &Observing<'_, Self::Message>) {}

    /// Asked before this layout is left. See [`Page::leaving`].
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    /// Returns [`Envelope`], not `Self::Message`, and this is the one place a
    /// layout differs from a page.
    ///
    /// A layout's tree contains its child's, and the child's messages are
    /// already sealed - they belong to the child. Elm's usual answer is for
    /// the parent to wrap them in a variant of its own, which is the tax this
    /// design refuses. So the seam is drawn once, here: `cx.mine(..)` seals
    /// the layout's own widgets, `cx.outlet()` hands over the child's already
    /// sealed, and both sides are then the same type.
    fn view<'a>(&'a self, cx: &LayoutCx<'a, Self>) -> Element<'a, Envelope>;
}

/// Where a node says what it watches.
pub struct Observing<'a, Message> {
    ctx: &'a FeatureInitContext,
    cursor: usize,
    deliver: Deliver,
    message: PhantomData<fn() -> Message>,
}

impl<'a, Message: Send + 'static> Observing<'a, Message> {
    fn new(ctx: &'a FeatureInitContext, deliver: Deliver) -> Self {
        Self {
            // The chain position of the segment being installed: everything
            // above it is already built, and nothing below exists yet.
            cursor: ctx.ancestors.len(),
            ctx,
            deliver,
            message: PhantomData,
        }
    }

    /// Start translating `R`'s updates into this node's messages.
    ///
    /// The rule between two states that reference each other instead of
    /// nesting. A selected row is an index into a list the domain owns; when
    /// the domain replaces that list the index may point past the end, and no
    /// lens over either state can derive that - only the update itself says
    /// whether the list was replaced or one row renamed.
    ///
    /// `translate` is a plain `fn`, not a closure: it has nothing to capture
    /// because it may not touch anything. It produces a message, and the
    /// node's `update` remains the single place state changes. `None` means
    /// this particular change is none of the node's business.
    ///
    /// The subscription belongs to the segment's scope, so the rule dies with
    /// the node that declared it.
    pub fn on<R: Reducer>(&self, translate: fn(&R::Update) -> Option<Message>) {
        let cursor = self.cursor;
        let deliver = self.deliver;
        self.ctx.observe::<R>(move |update| {
            if let Some(message) = translate(update) {
                envelope::park(Envelope::new(cursor, deliver, Box::new(message)));
            }
        });
    }
}

/// A feature's state and its contract with its actor.
///
/// One name, two halves, because there are two roles and no single name covers
/// both. A view keeps the first and drops the second; an `update` usually
/// wants both.
///
/// A snapshot rather than a borrow: a reducer is shared, lives in the scope
/// and changes from under the view, so the view holds the state as it was
/// when read, and a change goes to a copy. The node's own state is the
/// opposite case - it is borrowed, see [`Nodes`].
pub type Feature<R> = (Rc<R>, Dispatch);

fn feature_of<R>(props: &SegmentProps<Iced>) -> Feature<R>
where
    R: Reducer,
{
    let binding = props.binding::<R>();
    (binding.get(), binding.dispatch())
}

/// What a page's `view` is handed.
///
/// Carries the page type, not because a view needs it, but because reading
/// does: what a segment may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, P> {
    props: SegmentProps<Iced>,
    page: PhantomData<fn() -> P>,
    borrow: PhantomData<&'a Nodes>,
}

impl<P: Segment> PageCx<'_, P> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// Which feature that is, is settled at build time: this page installed it,
    /// or a segment above listed `R` in its `Exports`. Neither is true and it
    /// does not compile, rather than finding nothing on the first frame.
    ///
    /// The `_` is [`Reaches`]'s index, which says which of several impls
    /// applied. Rust has no partial turbofish, so it has to be written.
    pub fn state<R, I>(&self) -> Feature<R>
    where
        R: Reducer + Clone,
        P: Reaches<R, I>,
    {
        feature_of::<R>(&self.props)
    }
}

/// What a layout's `view` is handed: the same reading as a page, plus the
/// child and the seam between the two message types.
pub struct LayoutCx<'a, L: Layout> {
    props: SegmentProps<Iced>,
    nodes: &'a Nodes,
    layout: PhantomData<fn() -> L>,
}

impl<'a, L: Layout> LayoutCx<'a, L> {
    /// See [`PageCx::state`].
    pub fn state<R, I>(&self) -> Feature<R>
    where
        R: Reducer + Clone,
        L: Reaches<R, I>,
    {
        feature_of::<R>(&self.props)
    }

    /// Seals widgets that speak this layout's own message.
    pub fn mine(&self, element: impl Into<Element<'a, L::Message>>) -> Element<'a, Envelope> {
        let cursor = self.props.cursor;
        element
            .into()
            .map(move |message| Envelope::new(cursor, deliver_layout::<L>, Box::new(message)))
    }

    /// The next segment down the chain, for the layout to place where it
    /// wants. It borrows from the same store this layout does, which is what
    /// makes the whole tree one borrow rather than a chain of temporaries.
    pub fn outlet(&self) -> Element<'a, Envelope> {
        self.props.outlet(self.nodes)
    }

    /// Whether the segment directly below is `P` - what a tab strip needs to
    /// highlight the current tab without keeping a copy of the route.
    pub fn child_is<P: 'static>(&self) -> bool {
        self.props
            .chain
            .get(self.props.cursor + 1)
            .is_some_and(|entry| (entry.type_id)() == TypeId::of::<P>())
    }
}

/// What a node's `update` is handed: reading, acting, and navigating.
///
/// Carries the node type for the same reason [`PageCx`] does - `update` reads
/// exactly what `view` may.
pub struct UpdateCx<'a, S> {
    props: &'a SegmentProps<Iced>,
    segment: PhantomData<fn() -> S>,
}

impl<S: Segment> UpdateCx<'_, S> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// `emit` takes the action by value, so what is happening is readable
    /// where it happens: `dispatch.emit(Kill(pid))`. Nothing comes back - the
    /// answer arrives as an update to `R`, and from there as a message of this
    /// node's own if it said it was watching.
    ///
    /// See [`PageCx::state`] for what settles which feature answers.
    pub fn state<R, I>(&self) -> Feature<R>
    where
        R: Reducer + Clone,
        S: Reaches<R, I>,
    {
        feature_of::<R>(self.props)
    }
}

impl<S> UpdateCx<'_, S> {
    pub fn navigate<R>(&self) -> NavigateHandle<Iced, R>
    where
        R: RouteChain<Iced> + Clone + PartialEq + 'static,
    {
        nav::current::<R>()
    }
}

fn deliver_page<P: Page>(
    props: &SegmentProps<Iced>,
    nodes: &mut Nodes,
    message: Box<dyn Any + Send>,
) {
    envelope::deliver::<P, P::Message>(props, nodes, message, P::update, P::leaving);
}

fn deliver_layout<L: Layout>(
    props: &SegmentProps<Iced>,
    nodes: &mut Nodes,
    message: Box<dyn Any + Send>,
) {
    envelope::deliver::<L, L::Message>(props, nodes, message, L::update, L::leaving);
}

/// A zero-sized marker per segment type: what a `const` entry points at to get
/// its `&'static dyn Mount`.
pub struct MountPage<P>(pub PhantomData<P>);
pub struct MountLayout<L>(pub PhantomData<L>);

impl<P: Page> Mount<Iced> for MountPage<P> {
    fn view<'a>(&self, props: SegmentProps<Iced>, nodes: &'a Nodes) -> Element<'a, Envelope> {
        let cursor = props.cursor;
        let Some(page) = nodes.get::<P>(cursor) else {
            // A view asked for before the shell caught up with a navigation.
            // Drawing nothing beats a panic on a frame that is about to be
            // replaced anyway.
            return iced::widget::text("").into();
        };

        let cx = PageCx {
            props,
            page: PhantomData,
            borrow: PhantomData,
        };
        let _drawing = guinea_core::devtools::Rendering::of(std::any::type_name::<P>());
        page.view(&cx)
            .map(move |message| Envelope::new(cursor, deliver_page::<P>, Box::new(message)))
    }
}

impl<L: Layout> Mount<Iced> for MountLayout<L> {
    fn view<'a>(&self, props: SegmentProps<Iced>, nodes: &'a Nodes) -> Element<'a, Envelope> {
        let cursor = props.cursor;
        let Some(layout) = nodes.get::<L>(cursor) else {
            return iced::widget::text("").into();
        };

        let cx = LayoutCx {
            props,
            nodes,
            layout: PhantomData,
        };
        let _drawing = guinea_core::devtools::Rendering::of(std::any::type_name::<L>());
        layout.view(&cx)
    }
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<Iced> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        same_params::<P::Params>,
        &const { MountPage::<P>(PhantomData) },
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<Iced> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        same_params::<L::Params>,
        &const { MountLayout::<L>(PhantomData) },
        false,
    )
    .written(L::DECLARED)
}

/// Builds the node and parks it for the shell, and registers the guard that
/// will read its verdict.
///
/// The guard is registered on the scope, which the router asks, but it cannot
/// reach the shell's store - so what it reads is a slot beside the node, kept
/// current by every `update`.
fn stage_node<Node: Default + 'static>(
    ctx: &FeatureInitContext,
    node: Node,
    params: Box<dyn Any>,
    leaving: fn(&Node) -> Verdict,
    cache_state: bool,
) {
    let verdict = Rc::new(RefCell::new(leaving(&node)));

    let held = Held {
        node: Box::new(node),
        params,
        verdict: verdict.clone(),
    };

    ctx.on_leave(move || verdict.borrow().clone());
    nodes::stage(
        Placement {
            cursor: ctx.ancestors.len(),
            segment: TypeId::of::<Node>(),
        },
        held,
        cache_state,
    );
}

/// Hands what a segment installed to its scope - a feature's lifetime is the
/// segment's, and dropping this here would end it at the end of `install`.
fn own<T: 'static>(ctx: &FeatureInitContext, installed: T) {
    ctx.scope.own(guinea_core::scope::DropGuard(installed));
}

fn install_page<P: Page>(ctx: &FeatureInitContext, params: &dyn Any) -> anyhow::Result<()> {
    let params = narrow::<P::Params, P>(params)?;
    // Features first: `init` may read what they seeded, and an observer must
    // find the reducer it is being attached to.
    own(ctx, P::install(ctx, params)?);
    stage_node(
        ctx,
        P::init(ctx, params),
        Box::new(params.clone()),
        P::leaving,
        P::CACHE_STATE_IN_MEMORY,
    );
    P::observes(&Observing::new(ctx, deliver_page::<P>));
    Ok(())
}

fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &dyn Any) -> anyhow::Result<()> {
    let params = narrow::<L::Params, L>(params)?;
    own(ctx, L::install(ctx, params)?);
    stage_node(
        ctx,
        L::init(ctx, params),
        Box::new(params.clone()),
        L::leaving,
        false,
    );
    L::observes(&Observing::new(ctx, deliver_layout::<L>));
    Ok(())
}

/// A one-segment chain, for a page drawn without a route tree.
pub fn page_chain<P: Page>() -> &'static [SegmentEntry<Iced>] {
    single_entry_chain(segment_entry::<P>())
}

#[cfg(test)]
mod tests;
