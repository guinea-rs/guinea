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
mod lease;
mod run;

pub use keys::pressed;
pub use lease::{Lent, lend_terminal};
pub use run::{Flow, run};

use std::any::Any;

use guinea_app::feature::{FeatureInitContext, Reads};
use guinea_core::scope::Reducer;
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, Router, SegmentEntry, SegmentProps, Ui, single_entry_chain,
};
use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;

/// ratatui as a [`Ui`].
pub struct Tui;

impl Ui for Tui {
    type View<'a> = Node;
    /// Nothing: a terminal view draws from a snapshot inside the frame and
    /// holds no reference to state afterwards.
    type Nodes = ();
    type Mount = dyn TuiMount;
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

/// Whether a segment took the input it was offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

/// A leaf of the route tree, and its own state.
///
/// The struct that implements this **is** the page's state, as in the eframe
/// backend: what is typed in a filter, which row is picked. It is made by
/// [`Page::init`] when the page mounts and dropped when it leaves.
///
/// Input comes down the active chain from the leaf: [`Page::on_key`] and
/// [`Page::on_paste`] see it first, the layouts above see what it did not
/// take, and [`run`]'s `on_event` sees what nobody took.
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

    /// The state it starts with, when `Default` is not it. Runs once per
    /// mount, beside [`install`](Self::install).
    fn init(_ctx: &FeatureInitContext, _params: &Self::Params) -> Self {
        Self::default()
    }

    /// Draws the page into the frame it is handed.
    ///
    /// `render` and not `view`: ratatui is immediate, so this is not a
    /// description of what the page is - it is the drawing itself, run again
    /// for every frame.
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>);

    /// A key pressed while this page is mounted. Releases and repeats are not
    /// offered.
    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, _key: &KeyEvent) -> Handled {
        Handled::No
    }

    /// Text pasted while this page is mounted, where the terminal reports a
    /// paste as one.
    fn on_paste(&mut self, _cx: &mut InputCx<'_, Self>, _text: &str) -> Handled {
        Handled::No
    }
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

    fn render(&mut self, cx: &mut LayoutCx<'_, '_, Self>);

    /// See [`Page::on_key`]: offered what the segments below did not take.
    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, _key: &KeyEvent) -> Handled {
        Handled::No
    }

    /// See [`Page::on_paste`].
    fn on_paste(&mut self, _cx: &mut InputCx<'_, Self>, _text: &str) -> Handled {
        Handled::No
    }
}

/// What a segment entry points at here: its drawing, and its input.
pub trait TuiMount: Mount<Tui> {
    fn offer(&self, props: SegmentProps<Tui>, nav: &dyn Any, event: &Event) -> Handled;
}

/// Offers `event` to the active chain, leaf first, until a segment takes it.
pub(crate) fn offer(router: &Router<Tui>, nav: &dyn Any, event: &Event) -> Handled {
    let (Some(chain), Some(scopes)) = (router.active_chain(), router.active_scopes()) else {
        return Handled::No;
    };

    for cursor in (0..chain.len().min(scopes.len())).rev() {
        let props = SegmentProps {
            chain,
            scopes: scopes.clone(),
            cursor,
        };

        if chain[cursor].mount.offer(props, nav, event) == Handled::Yes {
            return Handled::Yes;
        }
    }

    Handled::No
}

enum Input<'a> {
    Key(&'a KeyEvent),
    Paste(&'a str),
}

fn input(event: &Event) -> Option<Input<'_>> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(Input::Key(key)),
        Event::Paste(text) => Some(Input::Paste(text)),
        _ => None,
    }
}

pub const fn segment_entry<P: Page>() -> SegmentEntry<Tui> {
    SegmentEntry::new::<P>(
        install_page::<P>,
        guinea_router::router::same_params::<P::Params>,
        <P::Installs as guinea_app::feature::Lists>::list,
        &const { MountPage::<P>(std::marker::PhantomData) } as &dyn TuiMount,
        P::CACHE_STATE_IN_MEMORY,
    )
    .written(P::DECLARED)
}

pub const fn layout_entry<L: Layout>() -> SegmentEntry<Tui> {
    SegmentEntry::new::<L>(
        install_layout::<L>,
        guinea_router::router::same_params::<L::Params>,
        <L::Installs as guinea_app::feature::Lists>::list,
        &const { MountLayout::<L>(std::marker::PhantomData) } as &dyn TuiMount,
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

impl<P: Page> Mount<Tui> for MountPage<P> {
    fn view<'a>(&self, props: SegmentProps<Tui>, _nodes: &'a ()) -> Node {
        let at = props.scopes[props.cursor];

        Node::new(move |frame, area| {
            let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
            guinea_router::mounted::with::<P, _>(&at, |page| {
                page.render(&mut PageCx {
                    frame,
                    area,
                    props,
                    page: std::marker::PhantomData,
                })
            })
        })
    }
}

impl<P: Page> TuiMount for MountPage<P> {
    fn offer(&self, props: SegmentProps<Tui>, nav: &dyn Any, event: &Event) -> Handled {
        let Some(input) = input(event) else {
            return Handled::No;
        };
        let at = props.scopes[props.cursor];
        let mut cx = InputCx {
            props,
            nav,
            segment: std::marker::PhantomData,
        };

        guinea_router::mounted::with::<P, _>(&at, |page| match input {
            Input::Key(key) => page.on_key(&mut cx, key),
            Input::Paste(text) => page.on_paste(&mut cx, text),
        })
    }
}

impl<L: Layout> TuiMount for MountLayout<L> {
    fn offer(&self, props: SegmentProps<Tui>, nav: &dyn Any, event: &Event) -> Handled {
        let Some(input) = input(event) else {
            return Handled::No;
        };
        let at = props.scopes[props.cursor];
        let mut cx = InputCx {
            props,
            nav,
            segment: std::marker::PhantomData,
        };

        guinea_router::mounted::with::<L, _>(&at, |layout| match input {
            Input::Key(key) => layout.on_key(&mut cx, key),
            Input::Paste(text) => layout.on_paste(&mut cx, text),
        })
    }
}

impl<L: Layout> Mount<Tui> for MountLayout<L> {
    fn view<'a>(&self, props: SegmentProps<Tui>, _nodes: &'a ()) -> Node {
        let at = props.scopes[props.cursor];

        Node::new(move |frame, area| {
            let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
            guinea_router::mounted::with::<L, _>(&at, |layout| {
                layout.render(&mut LayoutCx {
                    frame,
                    area,
                    props,
                    layout: std::marker::PhantomData,
                })
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

impl<P> PageCx<'_, '_, P> {
    /// The reducer's state and actions.
    ///
    /// No subscription, unlike the reactor's: a terminal redraws the whole
    /// frame on its own schedule, so there is nothing to invalidate - the next
    /// pass reads the state again.
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

impl<L> LayoutCx<'_, '_, L> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(&self.props)
    }
}

/// What a segment's input handler is handed: the state it may read, the
/// actions it may ask for, and the navigator.
pub struct InputCx<'a, S> {
    props: SegmentProps<Tui>,
    nav: &'a dyn Any,
    segment: std::marker::PhantomData<fn() -> S>,
}

impl<S> InputCx<'_, S> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (std::rc::Rc<R>, guinea_core::feature::Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(&self.props)
    }

    /// The reducer's actions, without its state.
    pub fn dispatch<R>(&self) -> guinea_core::feature::Dispatch
    where
        R: Reducer,
    {
        self.props.binding::<R>().dispatch()
    }

    /// The navigator [`run`] was started with. Panics when `R` is not the
    /// route type it was given.
    pub fn navigate<R>(&self) -> NavigateHandle<Tui, R>
    where
        R: RouteChain<Tui> + Clone + PartialEq + 'static,
    {
        self.nav
            .downcast_ref::<NavigateHandle<Tui, R>>()
            .unwrap_or_else(|| {
                panic!(
                    "navigate::<{}>() but run() was given a different route type",
                    std::any::type_name::<R>()
                )
            })
            .clone()
    }
}

impl<S> Reads for InputCx<'_, S> {
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

impl<P> Reads for PageCx<'_, '_, P> {
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

impl<L> Reads for LayoutCx<'_, '_, L> {
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
    use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    #[derive(Default)]
    struct Shell {
        quitting: bool,
    }

    impl Layout for Shell {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn render(&mut self, cx: &mut LayoutCx<'_, '_, Self>) {
            let chunks = RLayout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(cx.area());

            let chrome = if self.quitting { "quitting" } else { "tabs" };
            cx.frame().render_widget(Paragraph::new(chrome), chunks[0]);
            cx.outlet(chunks[1]);
        }

        fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('x') => {
                    self.quitting = true;
                    Handled::Yes
                }
                _ => Handled::No,
            }
        }
    }

    #[derive(Default)]
    struct Processes {
        frames: u32,
        filter: String,
    }

    impl Page for Processes {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
            self.frames += 1;

            let area = cx.area();
            let shown = format!("processes {} {}", self.frames, self.filter);
            cx.frame().render_widget(Paragraph::new(shown.trim_end().to_string()), area);
        }

        fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
            match key.code {
                KeyCode::Char(typed @ ('x' | 'y')) => {
                    self.filter.push(typed);
                    Handled::Yes
                }
                _ => Handled::No,
            }
        }

        fn on_paste(&mut self, _cx: &mut InputCx<'_, Self>, text: &str) -> Handled {
            self.filter.push_str(text);
            Handled::Yes
        }
    }

    const CHAIN: [SegmentEntry<Tui>; 2] = [layout_entry::<Shell>(), segment_entry::<Processes>()];

    struct Mounted {
        router: Router<Tui>,
        terminal: Terminal<TestBackend>,
    }

    impl Mounted {
        fn new() -> Self {
            let token = UiThreadToken::dangerously_create_token_unchecked();
            let router = Router::<Tui>::new(FeatureHost::detached(token));
            router
                .activate(&CHAIN, vec![Box::new(()), Box::new(())])
                .expect("activate");

            Self {
                router,
                terminal: Terminal::new(TestBackend::new(20, 3)).expect("terminal"),
            }
        }

        fn draw(&mut self) -> String {
            let router = &self.router;
            self.terminal
                .draw(|frame| router.render(&()).draw(frame, frame.area()))
                .expect("draw");

            let buffer = self.terminal.backend().buffer();
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
                .trim_end()
                .to_string()
        }

        fn offer(&self, event: Event) -> Handled {
            offer(&self.router, &(), &event)
        }
    }

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn release(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release))
    }

    #[test]
    fn a_layout_places_the_page_it_wraps() {
        assert_eq!(Mounted::new().draw(), "tabs\nprocesses 1");
    }

    #[test]
    fn a_page_keeps_its_own_state_between_frames() {
        let mut app = Mounted::new();
        app.draw();

        assert_eq!(app.draw(), "tabs\nprocesses 2");
    }

    #[test]
    fn a_key_is_offered_to_the_page_before_the_layout() {
        let mut app = Mounted::new();
        app.draw();

        assert_eq!(app.offer(press(KeyCode::Char('x'))), Handled::Yes);
        assert_eq!(app.draw(), "tabs\nprocesses 2 x");
    }

    #[test]
    fn a_key_the_page_leaves_reaches_the_layout() {
        let mut app = Mounted::new();
        app.draw();

        assert_eq!(app.offer(press(KeyCode::Char('q'))), Handled::Yes);
        assert_eq!(app.draw(), "quitting\nprocesses 2");
    }

    #[test]
    fn a_key_nobody_takes_is_left_for_the_application() {
        let mut app = Mounted::new();
        app.draw();

        assert_eq!(app.offer(press(KeyCode::Char('z'))), Handled::No);
        assert_eq!(app.draw(), "tabs\nprocesses 2");
    }

    #[test]
    fn a_release_is_not_offered() {
        let mut app = Mounted::new();
        app.draw();

        assert_eq!(app.offer(release(KeyCode::Char('x'))), Handled::No);
        assert_eq!(app.draw(), "tabs\nprocesses 2");
    }

    #[test]
    fn a_paste_is_offered_as_one_text() {
        let mut app = Mounted::new();
        app.draw();

        assert_eq!(app.offer(Event::Paste("db-01".into())), Handled::Yes);
        assert_eq!(app.draw(), "tabs\nprocesses 2 db-01");
    }
}
