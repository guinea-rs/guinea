//! guinea on egui: the same navigator and features, drawn immediately.
//!
//! The closest relative among the backends is ratatui, not WinUI: egui is
//! immediate, so a view is not a tree that is kept and diffed - it is drawing
//! that happens inside one frame and leaves nothing behind. As there, what a
//! layout mounts comes back as a [`Node`]: drawing deferred until the layout
//! supplies the [`egui::Ui`] to draw into, which is what lets it decide where
//! its child goes.
//!
//! What differs from the terminal is who owns the loop. eframe owns it, the
//! way the reactor does under WinUI, so [`run`] hands it over and puts the
//! frame inside `eframe::App::ui`. And unlike a terminal, egui sleeps when
//! nothing happens - so work finished on another thread has to wake it, which
//! is what the UI loop's wake-up, a `request_repaint`, is for.

extern crate self as guinea_eframe;

mod run;

pub use run::{MAIN, Window, run};

/// What `install` is handed. Re-exported because `#[page]` writes signatures
/// that name it, and a page should not have to import it just for that.
pub use guinea_app::feature::{FeatureInitContext, Install};
/// Writes down what a page or a layout left out. See [`Page`].
pub use guinea_macros::{Routable, eframe_layout as layout, eframe_page as page};
/// Where a window is, and how to move it.
pub use guinea_router::navigator::{Moved, Navigator, Routable};

use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::feature::Reads;
use guinea_core::binding::ReducerBinding;
use guinea_core::feature::Dispatch;
use guinea_core::scope::{Reducer, Scope};
use guinea_router::mount::{Outlet, Pass, binding, navigator};

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

/// A leaf: what a layout mounts, and its own state.
///
/// The struct that implements this **is** the page's state, as in the WinUI
/// and iced backends. What differs is that immediate mode has no later: the
/// frame that sees the click is the frame that answers it, so there is no
/// message and no `update` - [`Page::render`] takes `&mut self` and writes
/// what it decided where it decided it.
///
/// What belongs here is what only this page has an opinion about: which row
/// is picked, which tab is open, what is typed in a filter. What crosses the
/// page - what a domain owns, what another page reads - is a reducer, and
/// reaches this page through [`PageCx::read`].
///
/// `#[page]` writes what is left out: `Params` as `()`, and the `install`
/// that starts from `Default`.
pub trait Page: Sized + 'static {
    /// Where `impl Page` was written, for devtools to link to.
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the page is mounted with: `cx.mount::<ProcessPage>(&pid)`.
    ///
    /// `PartialEq` because a mount's one question about it is whether it is
    /// still the same: equal params are the same page in the same scope,
    /// other params another page in a scope of its own.
    type Params: PartialEq + Clone + 'static;

    /// Mounts the page: installs what it needs while it is mounted, and
    /// returns the state it starts with. Runs once per scope, when the scope
    /// is made.
    ///
    /// What it installs with `ctx.install::<F>(..)` is the page's scope's,
    /// and goes when the page does.
    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self>;

    /// Draws the page, and changes it. Runs again for every frame, so this is
    /// the drawing itself and not a description of it.
    fn render(&mut self, cx: &mut PageCx<'_, Self>);
}

/// A branch: draws its own chrome and mounts what goes under it. Its own
/// state, the same way a [`Page`] is.
pub trait Layout: Sized + 'static {
    /// Where `impl Layout` was written; see [`Page::DECLARED`].
    const DECLARED: Option<guinea_core::actor::shape::Declared> = None;

    /// What the layout is mounted with, as a page is: see [`Page::Params`].
    type Params: PartialEq + Clone + 'static;

    /// See [`Page::install`].
    ///
    /// What a layout installs is also what the pages below share: they come
    /// and go, and it stays. Whatever its features list in `Exports` those
    /// pages may read.
    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self>;

    fn render(&mut self, cx: &mut LayoutCx<'_, Self>);
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

/// How what was mounted into a scope draws, kept there as it mounts.
struct Draws(fn(Scope, &mut egui::Ui));

#[doc(hidden)]
pub fn install_page<P: Page>(ctx: &FeatureInitContext, params: &P::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Draws(page_draw::<P>));
    guinea_router::mount::written(scope, P::DECLARED);
    let node = P::install(ctx, params)?;
    scope.keep(node);
    Ok(())
}

#[doc(hidden)]
pub fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &L::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Draws(layout_draw::<L>));
    guinea_router::mount::written(scope, L::DECLARED);
    let node = L::install(ctx, params)?;
    scope.keep(node);
    Ok(())
}

fn page_draw<P: Page>(scope: Scope, ui: &mut egui::Ui) {
    let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<P>());
    scope.kept::<P, _>(|page| {
        page.render(&mut PageCx {
            ui,
            scope,
            page: PhantomData,
        })
    });
}

fn layout_draw<L: Layout>(scope: Scope, ui: &mut egui::Ui) {
    let _drawing = guinea_core::observability::Rendering::of(std::any::type_name::<L>());
    let pass = Pass::begin(scope);
    scope.kept::<L, _>(|layout| {
        layout.render(&mut LayoutCx {
            ui,
            scope,
            pass: pass.clone(),
            layout: PhantomData,
        })
    });
    pass.end();
}

/// The drawing of what was mounted into `scope`.
pub(crate) fn drawn(scope: Scope) -> Node {
    let draws = scope.keeps::<Draws>().then(|| scope.kept::<Draws, _>(|draws| draws.0)).flatten();
    Node::new(move |ui| {
        if let Some(draw) = draws {
            draw(scope, ui);
        }
    })
}

pub(crate) fn failed(error: &anyhow::Error) -> Node {
    let said = format!("{error:#}");
    Node::new(move |ui| {
        ui.label(said);
    })
}

/// What a layout mounted: the child's drawing, or why it did not mount.
///
/// Drawn as it is - `Node::from(cx.mount::<Services>(&())).draw(ui)` - or
/// handed to an [`outlet`](LayoutCx::outlet). One that failed draws what went
/// wrong in its place, and the others beside it stand.
#[must_use = "a mounted child shows only where it is drawn"]
pub struct Child {
    pass: Pass,
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

    /// Draws it into `ui`.
    pub fn draw(self, ui: &mut egui::Ui) {
        Node::from(self).draw(ui);
    }
}

impl From<Child> for Node {
    fn from(child: Child) -> Self {
        match child.mounted {
            Ok(scope) => drawn(scope),
            Err(error) => failed(&error),
        }
    }
}

fn feature_of<R: Reducer>(scope: Scope) -> (Rc<R>, Dispatch) {
    let binding = binding::<R>(scope);
    (binding.get(), binding.dispatch())
}

/// What a page's drawing is handed.
///
/// Carries the page type, not because drawing needs it, but because reading
/// does: what a page may read is a fact about where it sits, and this is
/// where that fact enters the signature.
pub struct PageCx<'a, P> {
    ui: &'a mut egui::Ui,
    scope: Scope,
    page: PhantomData<fn() -> P>,
}

impl<P> PageCx<'_, P> {
    /// The reducer's state and actions.
    ///
    /// No subscription, as in the terminal: egui redraws the whole frame, so
    /// there is nothing to invalidate - the next pass reads the state again.
    /// What does need saying is when a frame should happen at all, and that is
    /// the UI loop's job.
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
        binding::<R>(self.scope)
    }

    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    /// The navigator over `R` nearest above this page.
    ///
    /// A move made while drawing is drawn in the next frame: this one
    /// finishes with what it drew.
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }
}

impl<P> Reads for PageCx<'_, P> {
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
pub struct LayoutCx<'a, L> {
    ui: &'a mut egui::Ui,
    scope: Scope,
    pass: Pass,
    layout: PhantomData<fn() -> L>,
}

impl<L> LayoutCx<'_, L> {
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

    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    /// See [`PageCx::navigate`].
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
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
    /// Handed back rather than drawn here: a layout takes its `ui` from this
    /// same context, so a method that drew the child would need the context
    /// twice at once.
    pub fn mount<S: Mountable>(&mut self, params: &S::Params) -> Child {
        Child {
            pass: self.pass.clone(),
            mounted: self.pass.mount::<S, _>(params, |cx| S::install(cx, params)),
        }
    }

    /// What `nav` says is here, mounted by `draw`: the layout's `match` on
    /// its own level of the route.
    ///
    /// What it mounted is what the navigator asks before it moves on. A child
    /// that fails to mount puts the navigator back where it was, keeps what
    /// stood here before, and leaves why on the navigator's
    /// [`failure`](Navigator::failure).
    pub fn outlet<R: Routable>(&mut self, nav: &Navigator<R>, draw: impl FnOnce(&R, &mut Self) -> Child) -> Node {
        let pass = self.pass.clone();
        match pass.outlet(nav, |here| draw(here, self).mounted) {
            Outlet::Here(scope) | Outlet::Held(scope) => drawn(scope),
            Outlet::Failed(error) => failed(&error),
            Outlet::Nowhere => Node::new(|_| {}),
        }
    }
}

impl<L> Reads for LayoutCx<'_, L> {
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
