//! A backend that draws nothing.
//!
//! Not a stub kept alive for tests: it is the proof that [`Ui`] demands
//! nothing of a backend. Its view type is `()`, it has no render context worth
//! the name, and the router works with it unchanged - so any requirement that
//! quietly crept into the agnostic half would fail to compile here first.

use guinea_core::scope::Reducer;

use guinea_app::feature::{FeatureInitContext, Reaches, Segment};
use crate::router::{Mount, SegmentEntry, SegmentProps, Ui, single_entry_chain};

pub struct Headless;

impl Ui for Headless {
    type View<'a> = ();
    /// Nothing to borrow from: a view that draws nothing holds nothing.
    type Nodes = ();
}

/// A leaf, in a backend that renders nothing. Note how little it has in common
/// with `winui::Page` beyond the name - which is the reason neither of them
/// belongs in the router.
pub trait Page: Sized + 'static {
    const CACHE_STATE_IN_MEMORY: bool = false;

    /// Where `impl Page` was written; see the backends' own `Page`.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What this page captured from the route, named by `routes!`. `()` for a
    /// page that captures nothing.
    ///
    /// `PartialEq` because the router's one question about a capture is
    /// whether it is still the same one - which decides what reinstalls and
    /// which cached state may come back.
    type Params: PartialEq + 'static;

    /// What this page installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking, and one that is added has nowhere to go
    /// until it is declared. Drift is not caught here, it is impossible.
    ///
    /// What is returned is then owned by the segment's scope, which is what
    /// gives a feature its own lifetime: whatever it keeps in itself lives as
    /// long as the segment that installed it and dies with it.
    ///
    /// The price is that `install` has no default any more: Rust has no
    /// conditional default body, so "returns `()`" cannot be assumed for the
    /// segments that install nothing.
    type Installs: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    fn view(cx: &mut HeadlessCx<Self>);
}

pub trait Layout: Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What every page under this layout carries, derived by `routes!` as the
    /// intersection of their parameters. A layout declares nothing; it is
    /// handed what all of its children were reached with.
    type Params: PartialEq + 'static;

    /// What this layout installs. See [`Page::Installs`].
    type Installs: 'static;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs>;

    fn view(cx: &mut HeadlessCx<Self>);
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<Headless> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        crate::router::same_params::<P::Params>,
        &const { MountPage::<P>(std::marker::PhantomData) },
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<Headless> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        crate::router::same_params::<L::Params>,
        &const { MountLayout::<L>(std::marker::PhantomData) },
        false,
    )
    .written(L::DECLARED)
}

fn install_page<P: Page>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    own(ctx, P::install(ctx, crate::router::narrow::<P::Params, P>(params)?)?);
    Ok(())
}

/// Hands what a segment installed to its scope.
///
/// A feature is a unit with its own lifetime, and this is where it gets one:
/// dropping the returned value here would end that lifetime at the end of
/// `install`, taking with it anything the feature kept in itself and nothing
/// else happened to hold.
fn own<T: 'static>(ctx: &FeatureInitContext, installed: T) {
    ctx.scope.own(guinea_core::scope::DropGuard(installed));
}

fn install_layout<L: Layout>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    own(ctx, L::install(ctx, crate::router::narrow::<L::Params, L>(params)?)?);
    Ok(())
}

/// What a headless view gets: the segment it belongs to, and nothing else.
///
/// Carries the segment type because reading needs it: what a segment may read
/// is a fact about where it sits, and this is where that fact enters the
/// signature.
pub struct HeadlessCx<S> {
    props: SegmentProps<Headless>,
    segment: std::marker::PhantomData<fn() -> S>,
}

impl<S> HeadlessCx<S> {
    pub fn outlet(&self) {
        self.props.outlet(&())
    }
}

impl<S: Segment> HeadlessCx<S> {
    /// The reducer's current state and what may be asked of its actor. No
    /// subscription: with nothing to re-render, a change is observed by
    /// reading again.
    ///
    /// Which feature answers is settled at build time: this segment installed
    /// it, or a segment above listed it in `Exports`. The `_` is [`Reaches`]'s
    /// index, which says which of several impls applied - Rust has no partial
    /// turbofish, so it has to be written.
    ///
    /// Reaching for what a feature above kept to itself does not compile. It
    /// used to be a panic on the first render, which meant a page could be
    /// wrong for as long as nobody walked to it:
    ///
    /// ```compile_fail
    /// use guinea_app::feature::{Feature, FeatureInitContext, Segment};
    /// use guinea_core::scope::Reducer;
    /// use guinea_router::headless::{HeadlessCx, Layout, Page};
    ///
    /// #[derive(Default, Clone)]
    /// struct Hidden(u32);
    ///
    /// impl Reducer for Hidden {
    ///     type Update = u32;
    ///     fn reduce(&mut self, to: u32) { self.0 = to; }
    /// }
    ///
    /// struct Chrome;
    ///
    /// impl Feature for Chrome {
    ///     type Params = ();
    ///     type Exports = ();
    ///
    ///     fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
    ///         cx.state::<Hidden>().plain();
    ///         Ok(Self)
    ///     }
    /// }
    ///
    /// struct Shell;
    ///
    /// impl Layout for Shell {
    ///     type Params = ();
    ///     type Installs = Chrome;
    ///
    ///     fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
    ///         cx.install::<Chrome>(&())
    ///     }
    ///
    ///     fn view(cx: &mut HeadlessCx<Self>) { cx.outlet(); }
    /// }
    ///
    /// impl Segment for Shell {
    ///     type Installs = <Shell as Layout>::Installs;
    ///     type Above = ();
    /// }
    ///
    /// struct Prier;
    ///
    /// impl Page for Prier {
    ///     type Params = ();
    ///     type Installs = ();
    ///
    ///     fn install(_cx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> { Ok(()) }
    ///
    ///     fn view(cx: &mut HeadlessCx<Self>) {
    ///         // `Chrome` claimed `Hidden` and exported nothing.
    ///         let _ = cx.read::<Hidden, _>();
    ///     }
    /// }
    ///
    /// impl Segment for Prier {
    ///     type Installs = <Prier as Page>::Installs;
    ///     type Above = (Shell, ());
    /// }
    /// ```
    pub fn read<R, I>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
        S: Reaches<R, I>,
    {
        let binding = self.props.binding::<R>();
        (binding.get(), binding.dispatch())
    }
}

/// The `Mount` implementors: a zero-sized marker per segment type, which is
/// how a `const` entry gets a `&'static dyn Mount`.
pub struct MountPage<P>(pub std::marker::PhantomData<P>);
pub struct MountLayout<L>(pub std::marker::PhantomData<L>);

impl<P: Page> Mount<Headless> for MountPage<P> {
    fn view<'a>(&self, props: SegmentProps<Headless>, _nodes: &'a ()) {
        let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
        P::view(&mut HeadlessCx {
            props,
            segment: std::marker::PhantomData,
        })
    }
}

impl<L: Layout> Mount<Headless> for MountLayout<L> {
    fn view<'a>(&self, props: SegmentProps<Headless>, _nodes: &'a ()) {
        let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
        L::view(&mut HeadlessCx {
            props,
            segment: std::marker::PhantomData,
        })
    }
}

pub fn page_chain<P: Page>() -> &'static [SegmentEntry<Headless>] {
    single_entry_chain(segment_entry::<P>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_app::feature::{FeatureHost, FeatureInitContext};
    use crate::router::Router;
    use guinea_core::actor::UiThreadToken;
    use std::rc::Rc;

    /// Plain Rust: the state is the reducer, and the two items are both about
    /// state. Nothing drives it, and the type says so.
    #[derive(Clone, Default, PartialEq, Debug)]
    struct Counter {
        installs: u32,
    }

    impl Reducer for Counter {
        type Update = u32;

        fn reduce(&mut self, by: u32) {
            self.installs += by;
        }
    }

    struct Page1;

    impl Page for Page1 {
        type Params = ();
        type Installs = guinea_core::feature::Bound<Counter>;

        fn install(
            ctx: &FeatureInitContext,
            _params: &(),
        ) -> anyhow::Result<guinea_core::feature::Bound<Counter>> {
            let counter = ctx.state::<Counter>().plain();
            counter.push(1);
            Ok(counter)
        }

        fn view(cx: &mut HeadlessCx<Self>) {
            let (counter, _) = cx.read::<Counter, _>();
            assert_eq!(counter.installs, 1);
        }
    }

    impl guinea_app::feature::Segment for Page1 {
        type Installs = <Page1 as Page>::Installs;
        type Above = ();
    }

    #[test]
    fn route_hooks_see_the_previous_and_current_path() {
        use std::cell::RefCell;

        let token = UiThreadToken::dangerously_create_token_unchecked();
        let router = Rc::new(Router::<Headless>::new(FeatureHost::detached(token)));

        let seen: Rc<RefCell<Vec<(Option<String>, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let recorded = seen.clone();
        let handle = router.on_route_change(move |from, to| {
            recorded
                .borrow_mut()
                .push((from.map(str::to_string), to.to_string()));
        });

        router.route_changed("/a");
        router.route_changed("/b");

        // Dropping the handle is what a view unmounting does.
        drop(handle);
        router.route_changed("/c");

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "the hook stopped when its handle was dropped");
        assert_eq!(seen[0], (None, "/a".to_string()));
        assert_eq!(seen[1], (Some("/a".to_string()), "/b".to_string()));
    }

    #[test]
    fn the_router_runs_with_a_backend_that_draws_nothing() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let router = Router::<Headless>::new(FeatureHost::detached(token));

        let scope = router
            .activate(page_chain::<Page1>(), vec![Box::new(())])
            .expect("activate");

        assert_eq!(scope.state::<Counter>().borrow().installs, 1);

        // Rendering is a plain call here - there is no reconciler to hand the
        // mounted view to, which is exactly what makes this a useful check.
        router.render(&());
    }
}
