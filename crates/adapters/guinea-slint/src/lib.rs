//! guinea on Slint: the same router and features, in a tree Slint compiled.
//!
//! The other two backends hand the router a view: WinUI an element tree,
//! ratatui a piece of deferred drawing. Slint hands it nothing. Its tree is
//! declared in `.slint` and compiled, and a component written in Rust cannot
//! be embedded into another one at runtime - `ComponentContainer` exists, but
//! the code generator answers `todo!("Components written in Rust can not get
//! embedded yet")`, so a factory's product ends up in the window without ever
//! being joined to the tree above it. It looks like it works until a second
//! page is mounted and the item indices start to overlap.
//!
//! So here a segment renders nothing: `View = ()`. The whole tree exists from
//! the start, with the pages in it, and the route decides which branch is
//! alive - a `.slint` conditional, which Slint creates and destroys on its
//! own. What the router still does is everything it does elsewhere: install
//! the features a route needs, own their scopes, and tear them down on the way
//! out.
//!
//! State reaches the tree through globals, one per page, declared beside the
//! component that reads them. A global is a singleton, so a binding made when
//! the page was installed survives the branch being destroyed and rebuilt -
//! which is exactly what happens every time the route changes.

mod convert;
mod dispatcher;
mod model;
mod monitors;
mod nav;
mod question;
mod root;
mod run;
mod windows;

pub use convert::ToSlint;
pub use guinea_core::guard::Ask;
pub use question::{answer, on_question};
pub use run::{MAIN, run};

use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::feature::{FeatureInitContext, Reaches, Segment};
use guinea_core::binding::ReducerBinding;
use guinea_core::scope::{DropGuard, Reducer};
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, SegmentEntry, SegmentProps, Ui, single_entry_chain,
};

/// Installing a root without [`run`], for tests and for an application that
/// drives Slint's loop itself.
pub mod testing {
    /// Parks a root the way [`crate::run`] does, so that pages can be
    /// installed and bound with no window loop running.
    pub fn set_root<W: slint::ComponentHandle + 'static>(root: W) {
        crate::root::install(root)
    }
}

/// Slint as a [`Ui`].
pub struct Slint;

impl Ui for Slint {
    type View<'a> = ();
    /// Nothing: Slint owns its own tree and this backend pushes into it, so
    /// there is no view here to borrow from anything.
    type Nodes = ();
}

/// A leaf of the route tree.
///
/// There is no `view`, and the name would lie if there were: the view is the
/// `.slint` file. What Rust adds is the wiring.
pub trait Page: Sized + 'static {
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
    type Installs: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    /// Ties this page's globals to state and actions.
    ///
    /// Runs when the router installs the page, which is before anything is
    /// drawn, and again on every later install. The globals outlive the
    /// branch that reads them, so nothing here has to be redone when Slint
    /// rebuilds it.
    fn bind(cx: PageCx<Self>);
}

/// A branch: the component the pages below it sit inside.
pub trait Layout: Sized + 'static {
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
    type Installs: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    fn bind(cx: LayoutCx<Self>);
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<Slint> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        guinea_router::router::same_params::<P::Params>,
        &NothingToRender,
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<Slint> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        guinea_router::router::same_params::<L::Params>,
        &NothingToRender,
        false,
    )
    .written(L::DECLARED)
}

/// Wiring happens when a segment is installed, not when it is mounted.
///
/// Mounting is per-render, and the router re-mounts a whole chain every time.
/// Everywhere else that is what drawing means; here it would re-run the
/// bindings - handing a global a second listener, and re-setting a callback
/// from inside the very callback that navigated, which Slint refuses with
/// "Callback Handler set while called". Installing happens once per segment
/// per route, which is exactly the life the bindings should have: the scope
/// they hang on is created and dropped with it.
fn install_page<P: Page>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    own(ctx, P::install(ctx, guinea_router::router::narrow::<P::Params, P>(params)?)?);
    P::bind(PageCx {
        at: Where::of(ctx),
        page: PhantomData,
    });
    Ok(())
}

/// Hands what a segment installed to its scope - a feature's lifetime is the
/// segment's, and dropping this here would end it at the end of `install`.
fn own<T: 'static>(ctx: &FeatureInitContext, installed: T) {
    ctx.scope.own(DropGuard(installed));
}

fn install_layout<L: Layout>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    own(ctx, L::install(ctx, guinea_router::router::narrow::<L::Params, L>(params)?)?);
    L::bind(LayoutCx {
        at: Where::of(ctx),
        layout: PhantomData,
    });
    Ok(())
}

/// One marker for both roles: nothing here is drawn, so a page and a layout
/// mount the same way - not at all.
pub struct NothingToRender;

impl Mount<Slint> for NothingToRender {
    fn view<'a>(&self, _props: SegmentProps<Slint>, _nodes: &'a ()) {}
}

/// A one-segment chain, for a page shown without a route tree.
pub fn page_chain<P: Page>() -> &'static [SegmentEntry<Slint>] {
    single_entry_chain(segment_entry::<P>())
}

/// The scope a segment was installed in, and the same lookup `SegmentProps`
/// does from it.
#[derive(Clone)]
struct Where {
    scope: guinea_core::scope::Scope,
}

impl Where {
    fn of(ctx: &FeatureInitContext) -> Self {
        Self { scope: ctx.scope }
    }

    fn binding<R: Reducer>(&self) -> ReducerBinding<R> {
        self.scope
            .owner_of::<R>()
            .unwrap_or_else(|| {
                panic!(
                    "binding {} here found no scope that owns it: this segment did not claim \
                     it, and no ancestor exported it",
                    std::any::type_name::<R>()
                )
            })
            .binding::<R>()
    }

    fn own<T: 'static>(&self, resource: T) {
        self.scope.own(DropGuard(resource))
    }
}

/// What a page's wiring is handed.
///
/// Carries the page type, not because the wiring needs it, but because reading
/// does: what a segment may read is a fact about where it sits, and this is
/// where that fact enters the signature. Every method below that reaches a
/// reducer carries the proof, since every one of them is a read.
pub struct PageCx<P> {
    at: Where,
    page: PhantomData<fn() -> P>,
}

impl<P> Clone for PageCx<P> {
    fn clone(&self) -> Self {
        Self {
            at: self.at.clone(),
            page: PhantomData,
        }
    }
}

impl<P> PageCx<P> {
    /// The application's root component, for the globals hanging off it.
    ///
    /// ```ignore
    /// let model = cx.root::<AppWindow>().global::<ProcessesModel>();
    /// ```
    ///
    /// Panics if called before [`run`] has a window, or with a different type
    /// than the one it was given.
    pub fn root<W: slint::ComponentHandle + 'static>(&self) -> W {
        root::current::<W>()
    }

    /// A navigator over the route type the application runs.
    pub fn navigate<R>(&self) -> NavigateHandle<Slint, R>
    where
        R: RouteChain<Slint> + Clone + PartialEq + 'static,
    {
        nav::current::<R>()
    }

    /// Drops `resource` when the router drops this page.
    pub fn own<T: 'static>(&self, resource: T) {
        self.at.own(resource)
    }
}

impl<P: Segment> PageCx<P> {
    /// The reducer's binding: state now, and a place to subscribe.
    ///
    /// Which feature answers is settled at build time: this page installed it,
    /// or a segment above listed it in `Exports`. The `_` is [`Reaches`]'s
    /// index, which says which of several impls applied - Rust has no partial
    /// turbofish, so it has to be written.
    pub fn binding<R: Reducer, I>(&self) -> ReducerBinding<R>
    where
        P: Reaches<R, I>,
    {
        self.at.binding::<R>()
    }

    /// A snapshot of the reducer's state, and what may be asked of the
    /// feature that owns it. Read once: what stays in step is [`Self::bind`].
    pub fn read<R, I>(&self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
        P: Reaches<R, I>,
    {
        let binding = self.binding::<R, I>();
        (binding.get(), binding.dispatch())
    }

    /// What may be asked of the actor driving `R`.
    pub fn dispatch<R: Reducer, I>(&self) -> guinea_core::feature::Dispatch
    where
        P: Reaches<R, I>,
    {
        self.binding::<R, I>().dispatch()
    }

    /// Applies the state now, and again after every change, for as long as
    /// this page's scope lives.
    pub fn bind<R: Reducer, I>(&self, apply: impl Fn(&R) + 'static)
    where
        P: Reaches<R, I>,
    {
        bind_to_scope(&self.binding::<R, I>(), apply)
    }

    /// [`Self::bind`], with the root handed back on every call.
    ///
    /// Which is what a binding needs, since the globals it writes to hang off
    /// the root and borrow it - so nothing can capture one. Holding the root
    /// instead is safe: it is a handle, and the window outlives every page.
    pub fn bind_to<R, W, I>(&self, root: &W, apply: impl Fn(&W, &R) + 'static)
    where
        R: Reducer,
        W: slint::ComponentHandle + 'static,
        P: Reaches<R, I>,
    {
        let root = root.clone_strong();
        self.bind::<R, I>(move |state| apply(&root, state))
    }

    /// A list property that reads the state instead of copying it.
    ///
    /// Set once, not on every change: the model answers from the reducer's
    /// own state and converts a row when Slint asks for that row - so a list
    /// of ten thousand costs a screenful, and adding one row costs one.
    ///
    /// ```ignore
    /// model.set_items(cx.rows::<Processes, _, _>(|state| &state.items));
    /// ```
    pub fn rows<R, T, I>(&self, select: fn(&R) -> &[T]) -> slint::ModelRc<T::Slint>
    where
        R: Reducer,
        T: ToSlint + 'static,
        T::Slint: Clone + 'static,
        P: Reaches<R, I>,
    {
        model::Rows::new(self.binding::<R, I>(), select)
    }
}

/// What a layout's wiring is handed. The same as a page's: with the tree
/// declared up front, a layout has nothing to place.
pub struct LayoutCx<L> {
    at: Where,
    layout: PhantomData<fn() -> L>,
}

impl<L> Clone for LayoutCx<L> {
    fn clone(&self) -> Self {
        Self {
            at: self.at.clone(),
            layout: PhantomData,
        }
    }
}

impl<L> LayoutCx<L> {
    pub fn root<W: slint::ComponentHandle + 'static>(&self) -> W {
        root::current::<W>()
    }

    pub fn navigate<R>(&self) -> NavigateHandle<Slint, R>
    where
        R: RouteChain<Slint> + Clone + PartialEq + 'static,
    {
        nav::current::<R>()
    }

    pub fn own<T: 'static>(&self, resource: T) {
        self.at.own(resource)
    }
}

impl<L: Segment> LayoutCx<L> {
    /// See [`PageCx::binding`].
    pub fn binding<R: Reducer, I>(&self) -> ReducerBinding<R>
    where
        L: Reaches<R, I>,
    {
        self.at.binding::<R>()
    }

    /// See [`PageCx::read`].
    pub fn read<R, I>(&self) -> (Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
        L: Reaches<R, I>,
    {
        let binding = self.binding::<R, I>();
        (binding.get(), binding.dispatch())
    }

    /// What may be asked of the actor driving `R`.
    pub fn dispatch<R: Reducer, I>(&self) -> guinea_core::feature::Dispatch
    where
        L: Reaches<R, I>,
    {
        self.binding::<R, I>().dispatch()
    }

    pub fn bind<R: Reducer, I>(&self, apply: impl Fn(&R) + 'static)
    where
        L: Reaches<R, I>,
    {
        bind_to_scope(&self.binding::<R, I>(), apply)
    }

    /// [`Self::bind`], with the root handed back on every call.
    pub fn bind_to<R, W, I>(&self, root: &W, apply: impl Fn(&W, &R) + 'static)
    where
        R: Reducer,
        W: slint::ComponentHandle + 'static,
        L: Reaches<R, I>,
    {
        let root = root.clone_strong();
        self.bind::<R, I>(move |state| apply(&root, state))
    }

    /// A list property that reads the state instead of copying it.
    pub fn rows<R, T, I>(&self, select: fn(&R) -> &[T]) -> slint::ModelRc<T::Slint>
    where
        R: Reducer,
        T: ToSlint + 'static,
        T::Slint: Clone + 'static,
        L: Reaches<R, I>,
    {
        model::Rows::new(self.binding::<R, I>(), select)
    }
}

fn bind_to_scope<R: Reducer>(binding: &ReducerBinding<R>, apply: impl Fn(&R) + 'static) {
    apply(&binding.peek());
    binding.on_change_owned(apply);
}
