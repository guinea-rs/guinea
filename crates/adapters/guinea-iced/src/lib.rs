//! guinea on iced: Elm inside a node, actors between nodes.
//!
//! The other four backends are told what to draw and have no opinion about
//! where a click goes. iced does: it is Elm, so every interaction is a value
//! of one `Message` type that comes back to one `update`. That is the whole
//! reason this adapter exists - it is the one that has to say what happens
//! when a tree of independent nodes meets a toolkit that wants a single
//! message type at the top.
//!
//! The answer is that a node keeps its own `State`, its own `Message` and its
//! own `update`, all on the one trait that makes it a page or a layout, and
//! nothing above it ever names any of them. A parent does not wrap a child's
//! messages, so adding a page costs no edit anywhere else - the composition
//! tax Elm normally charges (a variant, a match arm, a `map`, per child, per
//! level) is not paid at all. What the toolkit sees is [`Envelope`]: an opaque
//! carrier holding the message, the node's own delivery function, and the
//! scope the node was mounted into.
//!
//! What is left over is the part Elm has no answer for: two pieces of state
//! that reference each other rather than nest - a selected row and a list an
//! actor replaces. That is `cx.on`, and it is a translation, not a mutation:
//! it turns what happened elsewhere into this node's own message, so there is
//! still exactly one place the node's state changes.
//!
//! A layout mounts what goes under it from its view, as on every backend:
//! `cx.mount::<Services>(&())` beside its own widgets, and
//! `cx.outlet(&nav, |route, cx| ..)` for what a navigator says is here.
//!
//! ```ignore
//! #[page]
//! impl Page for Processes {
//!     type Message = Msg;
//!
//!     fn observes(cx: &Observing<'_, Msg>) { cx.on::<Listing>(list_replaced); }
//!     fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) { .. }
//!     fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Msg> { .. }
//! }
//! ```

extern crate self as guinea_iced;

mod envelope;
mod nodes;
mod run;
mod wakeups;

pub use envelope::Envelope;
pub use nodes::Nodes;
pub use run::{MAIN, Window, run};

/// What a node answers when asked whether it may be left.
pub use guinea_core::guard::{Ask, Verdict};
/// What `install` is handed. Re-exported because `#[page]` writes signatures
/// that name it, and a node should not have to import it just for that.
pub use guinea_app::feature::{FeatureInitContext, Install};
/// Writes down the associated types a node left out. See [`Page`].
pub use guinea_macros::{Routable, iced_layout as layout, iced_page as page};
/// Where a window is, and how to move it.
pub use guinea_router::navigator::{Moved, Navigator, Routable};

use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::feature::Reads;
use guinea_core::feature::Dispatch;
use guinea_core::scope::{Reducer, Scope};
use guinea_router::mount::{Outlet, Pass, binding, navigator};

use envelope::Deliver;
use nodes::Held;

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

/// A leaf: an Elm node a layout mounts.
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
/// beside it".
pub trait Page: Sized + 'static {
    /// Where `impl Page` was written, for devtools to link to.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the page is mounted with: `cx.mount::<ProcessPage>(&pid)`. `()`
    /// for a page that needs nothing, which `#[page]` writes when it is left
    /// out.
    ///
    /// `PartialEq` because a mount's one question about it is whether it is
    /// still the same: equal params are the same page in the same scope,
    /// other params another page in a scope of its own.
    type Params: Clone + PartialEq + 'static;

    /// What can happen to this node. `Send` because iced's message type is,
    /// and this one travels inside it.
    type Message: Send + 'static;

    /// Mounts the page: installs what it needs while it is mounted, and
    /// returns the node it starts as. `#[page]` writes one that starts from
    /// `Default` when it is left out.
    ///
    /// What it installs with `ctx.install::<F>(..)` is the page's scope's,
    /// and goes when the page does.
    ///
    /// Runs once per scope, when the scope is made. A page woken from sleep
    /// is not mounted again.
    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self>;

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
    /// Effects are messages to actors - `cx.read::<R>().1.emit(..)` - not
    /// values returned from here. That is the trade this framework makes
    /// against `Task`: an effect that crosses a node boundary is an actor's
    /// job, and one that does not is a state change.
    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    /// `cx` is `&mut` for reading through [`Reads`], which every backend
    /// shares and one of them reads through hooks.
    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Self::Message>;
}

/// A branch: an Elm node that also mounts what goes under it.
pub trait Layout: Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the layout is mounted with, as a page is: see [`Page::Params`].
    type Params: Clone + PartialEq + 'static;

    type Message: Send + 'static;

    /// Mounts the layout. See [`Page::install`].
    ///
    /// What a layout installs is also what the nodes below share: they come
    /// and go, and it stays. Whatever its features list in `Exports` those
    /// nodes may read.
    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self>;

    fn observes(_cx: &Observing<'_, Self::Message>) {}

    /// Asked before this layout is left. See [`Page::leaving`].
    fn leaving(&self) -> Verdict {
        Verdict::Allow
    }

    fn update(&mut self, message: Self::Message, cx: &mut UpdateCx<'_, Self>);

    /// Returns [`Envelope`], not `Self::Message`, and this is the one place a
    /// layout differs from a page.
    ///
    /// A layout's tree contains its children's, and their messages are
    /// already sealed - they belong to the children. Elm's usual answer is
    /// for the parent to wrap them in a variant of its own, which is the tax
    /// this design refuses. So the seam is drawn once, here: `cx.mine(..)`
    /// seals the layout's own widgets, what `cx.mount` and `cx.outlet` give
    /// back is sealed already, and both sides are then the same type.
    fn view<'a>(&'a self, cx: &mut LayoutCx<'a, Self>) -> Element<'a, Envelope>;
}

/// A page or a layout: what [`LayoutCx::mount`] mounts. `#[page]` and
/// `#[layout]` write it.
#[doc(hidden)]
pub trait Mountable: 'static {
    type Params: PartialEq + Clone + 'static;
    type Message: 'static;

    /// Installs it into the scope `cx` is for, and leaves its node there and
    /// how it draws.
    fn install(cx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<()>;
}

/// Where a node says what it watches.
pub struct Observing<'a, Message> {
    ctx: &'a FeatureInitContext,
    deliver: Deliver,
    message: PhantomData<fn() -> Message>,
}

impl<'a, Message: Send + 'static> Observing<'a, Message> {
    fn new(ctx: &'a FeatureInitContext, deliver: Deliver) -> Self {
        Self {
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
    /// The subscription belongs to the node's scope, so the rule dies with
    /// the node that declared it.
    pub fn on<R: Reducer>(&self, translate: fn(&R::Update) -> Option<Message>) {
        let scope = self.ctx.__scope();
        let deliver = self.deliver;
        self.ctx.observe::<R>(move |update| {
            if let Some(message) = translate(update) {
                envelope::park(Envelope::node(scope, deliver, Box::new(message)));
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

fn feature_of<R: Reducer>(scope: Scope) -> Feature<R> {
    let binding = binding::<R>(scope);
    (binding.get(), binding.dispatch())
}

/// What a page's `view` is handed.
///
/// Carries the page type, not because a view needs it, but because reading
/// does: what a page may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, P> {
    scope: Scope,
    page: PhantomData<fn() -> P>,
    borrow: PhantomData<&'a Nodes>,
}

impl<P> PageCx<'_, P> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// The one this page installed, or the nearest above that listed `R` in
    /// its `Exports`. A read that reaches nothing panics on the frame that
    /// makes it, naming what the page was mounted under.
    pub fn read<R>(&mut self) -> Feature<R>
    where
        R: Reducer + Clone,
    {
        feature_of::<R>(self.scope)
    }

    /// The navigator over `R` nearest above this page.
    ///
    /// Panics when none was opened above: a page that navigates where nothing
    /// navigates is a wiring mistake.
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }
}

impl<P> Reads for PageCx<'_, P> {
    fn read<R>(&mut self) -> Feature<R>
    where
        R: Reducer + PartialEq,
    {
        feature_of::<R>(self.scope)
    }

    fn dispatch<R>(&self) -> Dispatch
    where
        R: Reducer,
    {
        binding::<R>(self.scope).dispatch()
    }
}

impl<L: Layout> Reads for LayoutCx<'_, L> {
    fn read<R>(&mut self) -> Feature<R>
    where
        R: Reducer + PartialEq,
    {
        feature_of::<R>(self.scope)
    }

    fn dispatch<R>(&self) -> Dispatch
    where
        R: Reducer,
    {
        binding::<R>(self.scope).dispatch()
    }
}

/// What a layout's `view` is handed: the same reading as a page, mounting,
/// and the seam between the two message types.
pub struct LayoutCx<'a, L: Layout> {
    scope: Scope,
    pass: Pass,
    nodes: &'a Nodes,
    layout: PhantomData<fn() -> L>,
}

impl<'a, L: Layout> LayoutCx<'a, L> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> Feature<R>
    where
        R: Reducer + Clone,
    {
        feature_of::<R>(self.scope)
    }

    /// Seals widgets that speak this layout's own message.
    pub fn mine(&self, element: impl Into<Element<'a, L::Message>>) -> Element<'a, Envelope> {
        let scope = self.scope;
        element
            .into()
            .map(move |message| Envelope::node(scope, deliver_layout::<L>, Box::new(message)))
    }

    /// Mounts `S` - a page or a layout - under this layout, with `params`.
    ///
    /// Installed the first time this layout's view asks for it, in a scope of
    /// its own under the layout's, and the same one every frame after that
    /// while the params are equal. Other params are another child. A child
    /// the view stops asking for goes when the view returns - unless it was
    /// [`keep`](Child::keep)s, which puts it to sleep instead.
    ///
    /// One that fails to install leaves nothing behind, and draws why in its
    /// place.
    pub fn mount<S: Mountable>(&mut self, params: &S::Params) -> Child<'a> {
        let mounted = self.pass.mount::<S, _>(params, |cx| S::install(cx, params));
        if let Ok(scope) = &mounted {
            self.nodes.adopt(*scope);
        }

        Child {
            pass: self.pass.clone(),
            nodes: self.nodes,
            mounted,
        }
    }

    /// What `nav` says is here, mounted by `draw`: the layout's `match` on
    /// its own level of the route.
    ///
    /// What it mounted is what the navigator asks before it moves on - its
    /// leave guards, and everything under it. A child that fails to mount
    /// puts the navigator back where it was, keeps what stood here before,
    /// and leaves why on the navigator's
    /// [`failure`](guinea_router::navigator::Navigator::failure).
    pub fn outlet<R: Routable>(
        &mut self,
        nav: &Navigator<R>,
        draw: impl FnOnce(&R, &mut Self) -> Child<'a>,
    ) -> Element<'a, Envelope> {
        let pass = self.pass.clone();
        match pass.outlet(nav, |here| draw(here, self).mounted) {
            Outlet::Here(scope) | Outlet::Held(scope) => drawn(scope, self.nodes),
            Outlet::Failed(error) => failed(&error),
            Outlet::Nowhere => iced::widget::text("").into(),
        }
    }

    /// See [`PageCx::navigate`].
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }
}

/// What a layout mounted: the child's view, or why it did not mount.
///
/// Placed as it is - `cx.mount::<Services>(&()).into()` - or handed to an
/// [`outlet`](LayoutCx::outlet). One that failed draws what went wrong in its
/// place, and the others beside it stand.
#[must_use = "a mounted child shows only where its view is placed"]
pub struct Child<'a> {
    pass: Pass,
    nodes: &'a Nodes,
    mounted: Result<Scope, anyhow::Error>,
}

impl Child<'_> {
    /// Keeps the child asleep when a later frame leaves it out, instead of
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

impl<'a> From<Child<'a>> for Element<'a, Envelope> {
    fn from(child: Child<'a>) -> Self {
        match child.mounted {
            Ok(scope) => drawn(scope, child.nodes),
            Err(error) => failed(&error),
        }
    }
}

/// How what was mounted into a scope draws, kept there as it mounts.
struct Draws(for<'a> fn(Scope, &'a Nodes) -> Element<'a, Envelope>);

/// The view of what was mounted into `scope`.
pub(crate) fn drawn(scope: Scope, nodes: &Nodes) -> Element<'_, Envelope> {
    let draws = scope.keeps::<Draws>().then(|| scope.kept::<Draws, _>(|draws| draws.0)).flatten();
    match draws {
        Some(draw) => draw(scope, nodes),
        None => iced::widget::text("").into(),
    }
}

pub(crate) fn failed<'a>(error: &anyhow::Error) -> Element<'a, Envelope> {
    iced::widget::text(format!("{error:#}")).into()
}

/// What a node's `update` is handed: reading, acting, and navigating.
///
/// Carries the node type for the same reason [`PageCx`] does - `update` reads
/// exactly what `view` may.
pub struct UpdateCx<'a, S> {
    scope: Scope,
    node: PhantomData<fn() -> &'a S>,
}

impl<S> UpdateCx<'_, S> {
    /// The feature that owns `R` - its state, and what can be asked of it.
    ///
    /// `emit` takes the action by value, so what is happening is readable
    /// where it happens: `dispatch.emit(Kill(pid))`. Nothing comes back - the
    /// answer arrives as an update to `R`, and from there as a message of this
    /// node's own if it said it was watching.
    ///
    /// See [`PageCx::read`] for what settles which feature answers.
    pub fn read<R>(&self) -> Feature<R>
    where
        R: Reducer + Clone,
    {
        feature_of::<R>(self.scope)
    }

    /// See [`PageCx::navigate`].
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }
}

fn deliver_page<P: Page>(scope: Scope, nodes: &mut Nodes, message: Box<dyn Any + Send>) {
    envelope::deliver::<P, P::Message>(scope, nodes, message, P::update, P::leaving);
}

fn deliver_layout<L: Layout>(scope: Scope, nodes: &mut Nodes, message: Box<dyn Any + Send>) {
    envelope::deliver::<L, L::Message>(scope, nodes, message, L::update, L::leaving);
}

fn page_view<P: Page>(scope: Scope, nodes: &Nodes) -> Element<'_, Envelope> {
    let Some(page) = nodes.get::<P>(scope) else {
        return iced::widget::text("").into();
    };

    let mut cx = PageCx {
        scope,
        page: PhantomData,
        borrow: PhantomData,
    };
    let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
    page.view(&mut cx)
        .map(move |message| Envelope::node(scope, deliver_page::<P>, Box::new(message)))
}

fn layout_view<L: Layout>(scope: Scope, nodes: &Nodes) -> Element<'_, Envelope> {
    let Some(layout) = nodes.get::<L>(scope) else {
        return iced::widget::text("").into();
    };

    let pass = Pass::begin(scope);
    let mut cx = LayoutCx {
        scope,
        pass: pass.clone(),
        nodes,
        layout: PhantomData,
    };
    let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
    let element = layout.view(&mut cx);
    pass.end();

    element
}

/// Builds the node and leaves it in its scope for the store to adopt, and
/// registers the guard that will read its verdict.
///
/// The guard is registered on the scope, which the navigator asks, but it
/// cannot reach the shell's store - so what it reads is a slot beside the
/// node, kept current by every `update`.
///
/// A question already answered is asked again with a token of its own: the
/// answer to the last attempt is not the answer to this one.
fn stage_node<Node: 'static>(ctx: &FeatureInitContext, node: Node, leaving: fn(&Node) -> Verdict) {
    let verdict = Rc::new(RefCell::new(leaving(&node)));

    ctx.on_leave({
        let verdict = verdict.clone();
        move || match &*verdict.borrow() {
            Verdict::Ask(ask, decision) if decision.is_settled() => {
                Verdict::Ask(ask.clone(), guinea_core::guard::Decision::new())
            }
            kept => kept.clone(),
        }
    });

    nodes::stage(
        ctx.__scope(),
        Held {
            node: Box::new(node),
            verdict,
        },
    );
}

#[doc(hidden)]
pub fn install_page<P: Page>(ctx: &FeatureInitContext, params: &P::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Draws(page_view::<P>));
    guinea_router::mount::written(scope, P::DECLARED);
    let node = P::install(ctx, params)?;
    stage_node(ctx, node, P::leaving);
    P::observes(&Observing::new(ctx, deliver_page::<P>));
    Ok(())
}

#[doc(hidden)]
pub fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &L::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Draws(layout_view::<L>));
    guinea_router::mount::written(scope, L::DECLARED);
    let node = L::install(ctx, params)?;
    stage_node(ctx, node, L::leaving);
    L::observes(&Observing::new(ctx, deliver_layout::<L>));
    Ok(())
}

#[cfg(test)]
mod tests;
