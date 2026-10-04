//! guinea on ratatui: the same router and features, drawn in a terminal.
//!
//! The interesting difference from the WinUI backend is what a view *is*.
//! WinUI is retained: a view builds an element tree and hands it over.
//! ratatui is immediate: a view draws into a frame that only exists during
//! one pass, and nothing survives it. So here a view is neither of those - it
//! is a [`Node`], a piece of drawing deferred until someone supplies a frame
//! and an area.
//!
//! That indirection is what lets a layout place its child. `mount` receives
//! only [`SegmentProps`], never a frame, so a node cannot draw when it is
//! built; by the time the frame exists the layout has decided the geometry and
//! passes down whatever rectangle it wants the page to occupy.

mod dialog;
mod dispatcher;
mod keys;
mod run;

pub use keys::pressed;
pub use run::{Flow, run};

use guinea_app::feature::{FeatureInitContext, Reaches, Reads, Segment};
use guinea_core::scope::Reducer;
use guinea_router::router::{Mount, SegmentEntry, SegmentProps, Ui, single_entry_chain};
use ratatui::Frame;
use ratatui::layout::Rect;

/// ratatui as a [`Ui`].
pub struct Tui;

impl Ui for Tui {
    type View<'a> = Node;
    /// Nothing: a terminal view draws from a snapshot inside the frame and
    /// holds no reference to state afterwards.
    type Nodes = ();
}

/// Drawing that has not happened yet.
///
/// Boxed because a segment's drawing captures its own scope and props, and
/// `FnOnce` because a node is drawn exactly once per frame - the next frame
/// mounts fresh ones.
pub struct Node(Box<dyn FnOnce(&mut Frame, Rect)>);

impl Node {
    pub fn new(draw: impl FnOnce(&mut Frame, Rect) + 'static) -> Self {
        Self(Box::new(draw))
    }

    /// Draws into `area` of `frame`.
    pub fn draw(self, frame: &mut Frame, area: Rect) {
        (self.0)(frame, area)
    }
}

/// A leaf of the route tree.
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

    /// Draws the page into the frame it is handed.
    ///
    /// `render` and not `view`: ratatui is immediate, so this is not a
    /// description of what the page is - it is the drawing itself, run again
    /// for every frame.
    fn render(cx: &mut PageCx<'_, '_, Self>);
}

/// A branch: draws its own chrome and decides where its child goes.
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

    fn render(cx: &mut LayoutCx<'_, '_, Self>);
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<Tui> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        guinea_router::router::same_params::<P::Params>,
        &const { MountPage::<P>(std::marker::PhantomData) },
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<Tui> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        guinea_router::router::same_params::<L::Params>,
        &const { MountLayout::<L>(std::marker::PhantomData) },
        false,
    )
    .written(L::DECLARED)
}

fn install_page<P: Page>(
    ctx: &FeatureInitContext,
    params: &dyn std::any::Any,
) -> anyhow::Result<()> {
    own(ctx, P::install(ctx, guinea_router::router::narrow::<P::Params, P>(params)?)?);
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
    own(ctx, L::install(ctx, guinea_router::router::narrow::<L::Params, L>(params)?)?);
    Ok(())
}

/// A zero-sized marker per segment type: what a `const` entry points at to get
/// its `&'static dyn Mount`.
pub struct MountPage<P>(pub std::marker::PhantomData<P>);
pub struct MountLayout<L>(pub std::marker::PhantomData<L>);

impl<P: Page> Mount<Tui> for MountPage<P> {
    fn view<'a>(&self, props: SegmentProps<Tui>, _nodes: &'a ()) -> Node {
        Node::new(move |frame, area| {
            let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
            P::render(&mut PageCx {
                frame,
                area,
                props,
                page: std::marker::PhantomData,
            })
        })
    }
}

impl<L: Layout> Mount<Tui> for MountLayout<L> {
    fn view<'a>(&self, props: SegmentProps<Tui>, _nodes: &'a ()) -> Node {
        Node::new(move |frame, area| {
            let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
            L::render(&mut LayoutCx {
                frame,
                area,
                props,
                layout: std::marker::PhantomData,
            })
        })
    }
}

/// A one-segment chain, for a page drawn without a route tree.
pub fn page_chain<P: Page>() -> &'static [SegmentEntry<Tui>] {
    single_entry_chain(segment_entry::<P>())
}

/// What a page's view is handed: the frame it draws into, the rectangle it was
/// given, and a way to read the state its feature owns.
///
/// Carries the page type, not because drawing needs it, but because reading
/// does: what a segment may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, 'b, P> {
    frame: &'a mut Frame<'b>,
    area: Rect,
    props: SegmentProps<Tui>,
    page: std::marker::PhantomData<fn() -> P>,
}

impl<'b, P> PageCx<'_, 'b, P> {
    pub fn frame(&mut self) -> &mut Frame<'b> {
        self.frame
    }

    pub fn area(&self) -> Rect {
        self.area
    }
}

impl<P: Segment> PageCx<'_, '_, P> {
    /// The reducer's state and actions.
    ///
    /// No subscription, unlike the reactor's: a terminal redraws the whole
    /// frame on its own schedule, so there is nothing to invalidate - the next
    /// pass reads the state again.
    ///
    /// Which feature answers is settled at build time: this page installed it,
    /// or a segment above listed it in `Exports`. The `_` is [`Reaches`]'s
    /// index, which says which of several impls applied - Rust has no partial
    /// turbofish, so it has to be written.
    ///
    /// The state comes shared, not copied: reading it every frame costs a
    /// count, and a change made mid-frame goes to a copy.
    pub fn read<R, I>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
        P: Reaches<R, I>,
    {
        feature_of::<R>(&self.props)
    }
}

fn feature_of<R: Reducer>(
    props: &SegmentProps<Tui>,
) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch) {
    let binding = props.binding::<R>();
    (binding.get(), binding.dispatch())
}

/// What a layout's view is handed. Same as a page's, plus the child.
pub struct LayoutCx<'a, 'b, L> {
    frame: &'a mut Frame<'b>,
    area: Rect,
    props: SegmentProps<Tui>,
    layout: std::marker::PhantomData<fn() -> L>,
}

impl<L: Segment> LayoutCx<'_, '_, L> {
    /// See [`PageCx::read`].
    pub fn read<R, I>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
        L: Reaches<R, I>,
    {
        feature_of::<R>(&self.props)
    }
}

impl<P: Segment> Reads for PageCx<'_, '_, P> {
    type Segment = P;

    fn read<R, I>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
        P: Reaches<R, I>,
    {
        feature_of::<R>(&self.props)
    }

    fn dispatch<R, I>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
        P: Reaches<R, I>,
    {
        self.props.binding::<R>().dispatch()
    }
}

impl<L: Segment> Reads for LayoutCx<'_, '_, L> {
    type Segment = L;

    fn read<R, I>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer + PartialEq,
        L: Reaches<R, I>,
    {
        feature_of::<R>(&self.props)
    }

    fn dispatch<R, I>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
        L: Reaches<R, I>,
    {
        self.props.binding::<R>().dispatch()
    }
}

impl<'b, L> LayoutCx<'_, 'b, L> {
    pub fn frame(&mut self) -> &mut Frame<'b> {
        self.frame
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// Draws the next segment down the chain into `area`.
    ///
    /// The whole reason a view is deferred: the child was mounted before any
    /// frame existed, and only here is it known where it belongs.
    pub fn outlet(&mut self, area: Rect) {
        self.props.outlet(&()).draw(self.frame, area);
    }

    /// Whether the segment directly below is `P`.
    ///
    /// What a tab strip needs, and cheaper than it looks: the chain already
    /// says which page is mounted, so highlighting the current tab needs
    /// neither the route nor a copy of it in state - and a copy is a second
    /// thing to keep in step.
    pub fn child_is<P: 'static>(&self) -> bool {
        self.props
            .chain
            .get(self.props.cursor + 1)
            .is_some_and(|entry| (entry.type_id)() == std::any::TypeId::of::<P>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_app::feature::FeatureHost;
    use guinea_core::actor::UiThreadToken;
    use guinea_router::router::Router;
    use ratatui::layout::{Constraint, Direction, Layout as RLayout};
    use ratatui::widgets::Paragraph;
    use ratatui::{Terminal, backend::TestBackend};

    struct Shell;

    impl Layout for Shell {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn render(cx: &mut LayoutCx<'_, '_, Self>) {
            let chunks = RLayout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(cx.area());

            let area = cx.area();
            cx.frame().render_widget(Paragraph::new("tabs"), chunks[0]);
            let _ = area;
            cx.outlet(chunks[1]);
        }
    }

    struct Processes;

    impl Page for Processes {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn render(cx: &mut PageCx<'_, '_, Self>) {
            let area = cx.area();
            cx.frame().render_widget(Paragraph::new("processes"), area);
        }
    }

    const CHAIN: [SegmentEntry<Tui>; 2] = [layout_entry::<Shell>(), segment_entry::<Processes>()];

    fn rendered() -> String {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let router = Router::<Tui>::new(FeatureHost::detached(token));
        router
            .activate(&CHAIN, vec![Box::new(()), Box::new(())])
            .expect("activate");

        let mut terminal = Terminal::new(TestBackend::new(12, 3)).expect("terminal");
        terminal
            .draw(|frame| router.render(&()).draw(frame, frame.area()))
            .expect("draw");

        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_layout_places_the_page_it_wraps() {
        // The same router, the same chain, drawn by a backend with no widget
        // tree at all - the layout put its chrome on the first row and handed
        // the rest to the page.
        assert_eq!(rendered().trim_end(), "tabs\nprocesses");
    }
}
