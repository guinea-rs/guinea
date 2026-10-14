//! guinea on ratatui: the same navigator and features, drawn in a terminal.
//!
//! The interesting difference from the WinUI backend is what a view *is*.
//! WinUI is retained: a view builds an element tree and hands it over.
//! ratatui is immediate: a view draws into a frame that only exists during
//! one pass, and nothing survives it. So what a layout mounts comes back as a
//! [`Node`], a piece of drawing deferred until the layout supplies a frame and
//! the rectangle it wants the child to occupy.
//!
//! Input follows the drawing. A key goes to what the last frame drew, the
//! segment drawn last first and every layout after what it drew, until one
//! takes it.

extern crate self as guinea_ratatui;

mod dialog;
mod keys;
mod run;

pub use keys::pressed;
pub use run::{MAIN, Window, run};

/// What `install` is handed. Re-exported because `#[page]` writes signatures
/// that name it, and a page should not have to import it just for that.
pub use guinea_app::feature::{FeatureInitContext, Install};
/// Writes down what a page or a layout left out. See [`Page`].
pub use guinea_macros::{Route, ratatui_layout as layout, ratatui_page as page};
/// Where a window is, and how to move it.
pub use guinea_router::navigator::{Moved, Navigator, Route};

use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::feature::Reads;
use guinea_core::binding::ReducerBinding;
use guinea_core::feature::Dispatch;
use guinea_core::scope::{Reducer, Scope};
use guinea_router::mount::{Outlet, Pass, binding, navigator};
use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};

/// Drawing that has not happened yet.
///
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

/// A leaf: what a layout mounts, and its own state.
///
/// The struct that implements this **is** the page's state, as in the eframe
/// backend: what is typed in a filter, which row is picked. It is made by
/// [`Page::install`] when the page mounts and dropped when it goes.
///
/// Input comes to what was drawn: [`Page::on_key`] and [`Page::on_paste`]
/// see it first, the layouts above see what it did not take.
///
/// `#[page]` writes what is left out: `Params` and `Installs` as `()`, and
/// the `install` that starts from `Default`.
pub trait Page: Sized + 'static {
    /// Where `impl Page` was written, for devtools to link to.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the page is mounted with: `cx.mount::<ProcessPage>(&pid)`.
    ///
    /// `PartialEq` because a mount's one question about it is whether it is
    /// still the same: equal params are the same page in the same scope,
    /// other params another page in a scope of its own.
    type Params: PartialEq + Clone + 'static;

    /// What this page installs, and `()` when it installs nothing.
    ///
    /// The list is not written beside the body - it *is* the body's
    /// obligation: `install` returns it, so a feature that stops being
    /// installed stops type-checking.
    ///
    /// What is returned is owned by the page's scope, which is what gives a
    /// feature its own lifetime.
    type Installs: guinea_app::feature::Lists;

    /// Mounts the page: installs what it needs while it is mounted, and
    /// returns the state it starts with beside its
    /// [`Installs`](Self::Installs). Runs once per scope, when the scope is
    /// made.
    fn install(
        ctx: &FeatureInitContext,
        params: &Self::Params,
    ) -> anyhow::Result<(Self, Self::Installs)>;

    /// Draws the page into the frame it is handed.
    ///
    /// `render` and not `view`: ratatui is immediate, so this is not a
    /// description of what the page is - it is the drawing itself, run again
    /// for every frame.
    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>);

    /// A key pressed while this page is drawn. Releases and repeats are not
    /// offered.
    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, _key: &KeyEvent) -> Handled {
        Handled::No
    }

    /// Text pasted while this page is drawn, where the terminal reports a
    /// paste as one.
    fn on_paste(&mut self, _cx: &mut InputCx<'_, Self>, _text: &str) -> Handled {
        Handled::No
    }
}

/// A branch: draws its own chrome and mounts what goes under it. Its own
/// state, the same way a [`Page`] is.
pub trait Layout: Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the layout is mounted with, as a page is: see [`Page::Params`].
    type Params: PartialEq + Clone + 'static;

    /// What this layout installs. See [`Page::Installs`].
    ///
    /// For a layout it is also what the pages below share: they come and go,
    /// and what is installed here stays. Whatever its features list in
    /// `Exports` those pages may read.
    type Installs: guinea_app::feature::Lists;

    /// See [`Page::install`].
    fn install(
        ctx: &FeatureInitContext,
        params: &Self::Params,
    ) -> anyhow::Result<(Self, Self::Installs)>;

    fn render(&mut self, cx: &mut LayoutCx<'_, '_, Self>);

    /// See [`Page::on_key`]: offered what was drawn under it did not take.
    ///
    /// The root layout is the last to be asked, which makes it the place for
    /// what is the application's and not a page's: switching tabs, going
    /// back, [`closing`](InputCx::close).
    fn on_key(&mut self, _cx: &mut InputCx<'_, Self>, _key: &KeyEvent) -> Handled {
        Handled::No
    }

    /// See [`Page::on_paste`].
    fn on_paste(&mut self, _cx: &mut InputCx<'_, Self>, _text: &str) -> Handled {
        Handled::No
    }
}

/// A page or a layout: what [`LayoutCx::mount`] mounts. `#[page]` and
/// `#[layout]` write it.
#[doc(hidden)]
pub trait Mountable: 'static {
    type Params: PartialEq + Clone + 'static;

    /// Installs it into the scope `cx` is for, and keeps its state there and
    /// how it draws.
    fn install(cx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<()>;
}

/// How what was mounted into a scope draws and takes input, kept there as it
/// mounts.
#[derive(Clone, Copy)]
struct Segment {
    draw: fn(Scope, Scope, &mut Frame, Rect),
    offer: fn(Scope, Scope, &Input) -> Handled,
}

/// What a window keeps between its frames: what the last one drew, in the
/// order it drew it, and whether a segment closed it.
#[derive(Default)]
pub(crate) struct Shown {
    drawn: Vec<Scope>,
    closed: bool,
}

#[doc(hidden)]
pub fn install_page<P: Page>(ctx: &FeatureInitContext, params: &P::Params) -> anyhow::Result<()> {
    ctx.scope.keep(Segment {
        draw: page_draw::<P>,
        offer: page_offer::<P>,
    });
    guinea_router::mount::written(ctx.scope, P::DECLARED);
    let (node, installs) = P::install(ctx, params)?;
    ctx.scope.hold_installed(installs);
    ctx.scope.keep(node);
    Ok(())
}

#[doc(hidden)]
pub fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &L::Params) -> anyhow::Result<()> {
    ctx.scope.keep(Segment {
        draw: layout_draw::<L>,
        offer: layout_offer::<L>,
    });
    guinea_router::mount::written(ctx.scope, L::DECLARED);
    let (node, installs) = L::install(ctx, params)?;
    ctx.scope.hold_installed(installs);
    ctx.scope.keep(node);
    Ok(())
}

fn page_draw<P: Page>(_window: Scope, scope: Scope, frame: &mut Frame, area: Rect) {
    let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
    scope.kept::<P, _>(|page| {
        page.render(&mut PageCx {
            frame,
            area,
            scope,
            page: PhantomData,
        })
    });
}

fn layout_draw<L: Layout>(window: Scope, scope: Scope, frame: &mut Frame, area: Rect) {
    let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
    let pass = Pass::begin(scope);
    scope.kept::<L, _>(|layout| {
        layout.render(&mut LayoutCx {
            frame,
            area,
            scope,
            window,
            pass: pass.clone(),
            layout: PhantomData,
        })
    });
    pass.end();
}

fn page_offer<P: Page>(window: Scope, scope: Scope, input: &Input) -> Handled {
    let mut cx = InputCx {
        scope,
        window,
        segment: PhantomData,
    };
    scope
        .kept::<P, _>(|page| match input {
            Input::Key(key) => page.on_key(&mut cx, key),
            Input::Paste(text) => page.on_paste(&mut cx, text),
        })
        .unwrap_or(Handled::No)
}

fn layout_offer<L: Layout>(window: Scope, scope: Scope, input: &Input) -> Handled {
    let mut cx = InputCx {
        scope,
        window,
        segment: PhantomData,
    };
    scope
        .kept::<L, _>(|layout| match input {
            Input::Key(key) => layout.on_key(&mut cx, key),
            Input::Paste(text) => layout.on_paste(&mut cx, text),
        })
        .unwrap_or(Handled::No)
}

/// The drawing of what was mounted into `scope`, in the window `window`.
pub(crate) fn drawn(window: Scope, scope: Scope) -> Node {
    let segment = scope.keeps::<Segment>().then(|| scope.kept::<Segment, _>(|segment| *segment)).flatten();
    Node::new(move |frame, area| {
        let Some(segment) = segment else {
            return;
        };
        window.kept::<Shown, _>(|shown| shown.drawn.push(scope));
        (segment.draw)(window, scope, frame, area);
    })
}

pub(crate) fn failed(error: &anyhow::Error) -> Node {
    let said = format!("{error:#}");
    Node::new(move |frame, area| {
        frame.render_widget(Paragraph::new(said).wrap(Wrap { trim: true }), area);
    })
}

pub(crate) enum Input {
    Key(KeyEvent),
    Paste(String),
}

fn input(event: &Event) -> Option<Input> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(Input::Key(*key)),
        Event::Paste(text) => Some(Input::Paste(text.clone())),
        _ => None,
    }
}

/// Offers `event` to what the last frame of `window` drew, the segment drawn
/// last first, until one takes it.
pub(crate) fn offer(window: Scope, event: &Event) -> Handled {
    let Some(input) = input(event) else {
        return Handled::No;
    };
    let drawn = window.kept::<Shown, _>(|shown| shown.drawn.clone()).unwrap_or_default();

    for scope in drawn.into_iter().rev() {
        if !scope.is_alive() || !scope.keeps::<Segment>() {
            continue;
        }
        let Some(segment) = scope.kept::<Segment, _>(|segment| *segment) else {
            continue;
        };
        if (segment.offer)(window, scope, &input) == Handled::Yes {
            return Handled::Yes;
        }
    }

    Handled::No
}

/// Whether a segment of `window` closed it.
pub(crate) fn closed(window: Scope) -> bool {
    window.keeps::<Shown>() && window.kept::<Shown, _>(|shown| shown.closed).unwrap_or_default()
}

/// What a layout mounted: the child's drawing, or why it did not mount.
///
/// Drawn as it is - `cx.mount::<Services>(&()).draw(cx.frame(), area)` - or
/// handed to an [`outlet`](LayoutCx::outlet). One that failed draws what went
/// wrong in its place, and the others beside it stand.
#[must_use = "a mounted child shows only where it is drawn"]
pub struct Child {
    pass: Pass,
    window: Scope,
    mounted: Result<Scope, anyhow::Error>,
}

impl Child {
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

    /// Draws it into `area` of `frame`.
    pub fn draw(self, frame: &mut Frame, area: Rect) {
        Node::from(self).draw(frame, area);
    }
}

impl From<Child> for Node {
    fn from(child: Child) -> Self {
        match child.mounted {
            Ok(scope) => drawn(child.window, scope),
            Err(error) => failed(&error),
        }
    }
}

fn feature_of<R: Reducer>(scope: Scope) -> (Rc<R>, Dispatch) {
    let binding = binding::<R>(scope);
    (binding.get(), binding.dispatch())
}

/// What a page's drawing is handed: the frame it draws into, the rectangle it
/// was given, and a way to read the state its feature owns.
///
/// Carries the page type, not because drawing needs it, but because reading
/// does: what a page may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, 'b, P> {
    frame: &'a mut Frame<'b>,
    area: Rect,
    scope: Scope,
    page: PhantomData<fn() -> P>,
}

impl<'b, P> PageCx<'_, 'b, P> {
    pub fn frame(&mut self) -> &mut Frame<'b> {
        self.frame
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// The reducer's state and actions.
    ///
    /// No subscription, unlike the reactor's: a terminal redraws the whole
    /// frame on its own schedule, so there is nothing to invalidate - the next
    /// pass reads the state again.
    ///
    /// The feature that answers is the one this page installed, or the nearest
    /// above that listed `R` in `Exports`. A read that reaches nothing panics
    /// here, naming what the page was mounted under.
    ///
    /// The state comes shared, not copied: reading it every frame costs a
    /// count, and a change made mid-frame goes to a copy.
    pub fn read<R>(&mut self) -> (Rc<R>, Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(self.scope)
    }

    /// The reducer's binding: its state, and a push straight into it.
    ///
    /// For state the UI owns outright, claimed with `cx.state::<R>().plain()`.
    /// State a feature drives is not this: pushing into it goes behind the
    /// back of whatever answers for it. Use [`PageCx::read`] and emit.
    pub fn binding<R>(&self) -> ReducerBinding<R>
    where
        R: Reducer,
    {
        binding::<R>(self.scope)
    }

    /// The navigator over `R` nearest above this page.
    ///
    /// A move made while drawing is drawn in the next frame: this one
    /// finishes with what it drew.
    pub fn navigate<R: Route>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }
}

impl<P> Reads for PageCx<'_, '_, P> {
    fn read<R>(&mut self) -> (Rc<R>, Dispatch)
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

/// What a layout's drawing is handed: the same as a page's, and mounting.
pub struct LayoutCx<'a, 'b, L> {
    frame: &'a mut Frame<'b>,
    area: Rect,
    scope: Scope,
    window: Scope,
    pass: Pass,
    layout: PhantomData<fn() -> L>,
}

impl<'b, L> LayoutCx<'_, 'b, L> {
    pub fn frame(&mut self) -> &mut Frame<'b> {
        self.frame
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (Rc<R>, Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(self.scope)
    }

    /// See [`PageCx::binding`].
    pub fn binding<R>(&self) -> ReducerBinding<R>
    where
        R: Reducer,
    {
        binding::<R>(self.scope)
    }

    /// See [`PageCx::navigate`].
    pub fn navigate<R: Route>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }

    /// Mounts `S` - a page or a layout - under this layout, with `params`.
    ///
    /// Installed the first frame this layout asks for it, in a scope of its
    /// own under the layout's, and the same one every frame after that while
    /// the params are equal. Other params are another child. A child the
    /// layout stops asking for goes when its drawing returns - unless it was
    /// [`keep`](Child::keep)s, which puts it to sleep instead.
    ///
    /// Handed back rather than drawn here: the layout decides where it goes,
    /// with the frame it takes from this same context.
    pub fn mount<S: Mountable>(&mut self, params: &S::Params) -> Child {
        Child {
            pass: self.pass.clone(),
            window: self.window,
            mounted: self.pass.mount::<S, _>(params, |cx| S::install(cx, params)),
        }
    }

    /// What `nav` says is here, mounted by `mount`: the layout's `match` on
    /// its own level of the route.
    ///
    /// What it mounted is what the navigator asks before it moves on. A child
    /// that fails to mount puts the navigator back where it was, keeps what
    /// stood here before, and leaves why on the navigator's
    /// [`failure`](Navigator::failure).
    pub fn outlet<R: Route>(&mut self, nav: &Navigator<R>, mount: impl FnOnce(&R, &mut Self) -> Child) -> Node {
        let pass = self.pass.clone();
        let window = self.window;
        match pass.outlet(nav, |here| mount(here, self).mounted) {
            Outlet::Here(scope) | Outlet::Held(scope) => drawn(window, scope),
            Outlet::Failed(error) => failed(&error),
            Outlet::Nowhere => Node::new(|_, _| {}),
        }
    }
}

impl<L> Reads for LayoutCx<'_, '_, L> {
    fn read<R>(&mut self) -> (Rc<R>, Dispatch)
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

/// What a segment's input handler is handed: the state it may read, the
/// actions it may ask for, the navigator, and the window.
pub struct InputCx<'a, S> {
    scope: Scope,
    window: Scope,
    segment: PhantomData<fn() -> &'a S>,
}

impl<S> InputCx<'_, S> {
    /// See [`PageCx::read`].
    pub fn read<R>(&mut self) -> (Rc<R>, Dispatch)
    where
        R: Reducer,
    {
        feature_of::<R>(self.scope)
    }

    /// The reducer's actions, without its state.
    pub fn dispatch<R>(&self) -> Dispatch
    where
        R: Reducer,
    {
        binding::<R>(self.scope).dispatch()
    }

    /// See [`PageCx::binding`].
    pub fn binding<R>(&self) -> ReducerBinding<R>
    where
        R: Reducer,
    {
        binding::<R>(self.scope)
    }

    /// See [`PageCx::navigate`]. A move made here is drawn in the next frame.
    pub fn navigate<R: Route>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }

    /// Closes the window: [`run`] returns after this event.
    pub fn close(&self) {
        self.window.kept::<Shown, _>(|shown| shown.closed = true);
    }
}

impl<S> Reads for InputCx<'_, S> {
    fn read<R>(&mut self) -> (Rc<R>, Dispatch)
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

#[cfg(test)]
mod tests;
