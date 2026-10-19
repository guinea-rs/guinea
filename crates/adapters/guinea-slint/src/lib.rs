//! guinea on Slint: the same navigator and features, in a tree Slint compiled.
//!
//! The other backends draw what a layout mounts: WinUI an element tree, egui
//! and ratatui a piece of deferred drawing. Slint draws nothing from Rust. Its
//! tree is declared in `.slint` and compiled, and a component written in Rust
//! cannot be embedded into another one at runtime - `ComponentContainer`
//! exists, but the code generator answers `todo!("Components written in Rust
//! can not get embedded yet")`.
//!
//! So the tree is the application's markup, and the route decides which
//! branch of it is alive: `if Navigator.route == RouteId.processes:
//! Processes { }`. What Rust still does is what it does everywhere else: a
//! layout mounts the pages under it with the same `match` on the route, which
//! installs their features, owns their scopes, and tears them down on the way
//! out.
//!
//! The markup learns where the window is from a `Navigator` global that
//! `guinea-slint-build` writes from the application's `Route` enum - the
//! branch, each place's parameters, back and forward, a guard's question -
//! and moves the window through the callbacks on it.
//!
//! State reaches the tree through globals, one per page, declared beside the
//! component that reads them. A global is a singleton, so what a page bound
//! when it mounted survives Slint destroying and rebuilding the branch that
//! reads it.

extern crate self as guinea_slint;

mod convert;
mod dispatcher;
mod model;
mod monitors;
mod run;
mod windows;

pub use convert::{FromSlint, ToSlint};
pub use guinea_core::guard::Ask;
pub use run::{MAIN, Opened, Window, run};

/// What `install` is handed. Re-exported because `#[page]` writes signatures
/// that name it, and a page should not have to import it just for that.
pub use guinea_app::feature::{FeatureInitContext, Install};
/// Writes down what a page or a layout left out. See [`Page`].
pub use guinea_macros::{Routable, slint_layout as layout, slint_page as page};
/// Where a window is, and how to move it.
pub use guinea_router::navigator::{Moved, Navigator, Routable};

use std::any::Any;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_core::__private::ReducerBinding;
use guinea_core::feature::Dispatch;
use guinea_core::scope::{Reducer, Scope};
use guinea_router::mount::{Pass, binding, navigator};

/// A route the window's `Navigator` global shows: written by
/// `guinea-slint-build` beside that global, from the same enum.
///
/// [`show`](Self::show) runs after every move the window made, and
/// [`listen`](Self::listen) once, when the window opens.
pub trait Shows<W>: Routable {
    /// Puts where `nav` is on the window's `Navigator` global: the branch,
    /// that place's parameters, whether there is a back and a forward, the
    /// question a guard is waiting on, and why the last move failed.
    fn show(window: &W, nav: &Navigator<Self>);

    /// Has the global's callbacks move `nav`.
    fn listen(window: &W, nav: &Navigator<Self>);
}

/// A leaf: what a layout mounts.
///
/// There is no `view`, and the name would lie if there were: the view is the
/// `.slint` file. What Rust adds is what the page installs and how its
/// globals are tied to it.
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
    /// returns its own state. Runs once per scope, when the scope is made.
    ///
    /// What it installs with `cx.install::<F>(..)` is the page's scope's,
    /// and goes when the page does.
    fn install(cx: &Install<Self::Params>) -> anyhow::Result<Self>;

    /// Ties this page's globals to state and actions, once, right after
    /// [`install`](Self::install).
    ///
    /// The globals outlive the branch that reads them, so nothing here has to
    /// be redone when Slint rebuilds it.
    fn bind(&mut self, _cx: &BindCx<Self>) {}
}

/// A branch: mounts what goes under it.
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
    fn install(cx: &Install<Self::Params>) -> anyhow::Result<Self>;

    /// See [`Page::bind`].
    fn bind(&mut self, _cx: &BindCx<Self>) {}

    /// Mounts what is under this layout - the same `match` on the route the
    /// markup makes, so what is installed is what is shown.
    ///
    /// Runs when the layout mounts, and again after every move of the
    /// window's navigator.
    fn mount(&mut self, cx: &mut LayoutCx<'_, Self>);
}

/// A page or a layout: what [`LayoutCx::mount`] mounts. `#[page]` and
/// `#[layout]` write it.
#[doc(hidden)]
pub trait Mountable: 'static {
    type Params: PartialEq + Clone + 'static;

    /// Installs it into the scope `cx` is for, keeps its state there, and
    /// binds it.
    fn install(cx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<()>;
}

/// How what was mounted into a scope mounts what is under it, kept there as
/// it mounts.
#[derive(Clone, Copy)]
struct Segment(fn(Scope));

/// The window's component, kept in the window's scope for what binds to its
/// globals.
struct Component(Rc<dyn Any>);

#[doc(hidden)]
pub fn install_page<P: Page>(ctx: &FeatureInitContext, params: &P::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Segment(|_| {}));
    guinea_router::mount::written(scope, P::DECLARED);
    let mut page = P::install(&Install::new(ctx, params))?;
    page.bind(&BindCx {
        scope,
        segment: PhantomData,
    });
    scope.keep(page);
    Ok(())
}

#[doc(hidden)]
pub fn install_layout<L: Layout>(ctx: &FeatureInitContext, params: &L::Params) -> anyhow::Result<()> {
    let scope = ctx.__scope();
    scope.keep(Segment(layout_mount::<L>));
    guinea_router::mount::written(scope, L::DECLARED);
    let mut layout = L::install(&Install::new(ctx, params))?;
    layout.bind(&BindCx {
        scope,
        segment: PhantomData,
    });
    scope.keep(layout);
    Ok(())
}

fn layout_mount<L: Layout>(scope: Scope) {
    let pass = Pass::begin(scope);
    scope.kept::<L, _>(|layout| {
        layout.mount(&mut LayoutCx {
            scope,
            pass: pass.clone(),
            layout: PhantomData,
        })
    });
    pass.end();
}

/// Mounts what is under `scope`, as what was mounted into it says.
pub(crate) fn descend(scope: Scope) {
    let segment = scope.try_kept::<Segment, _>(|segment| *segment);
    if let Some(Segment(mount)) = segment {
        mount(scope);
    }
}

/// What a layout mounted, or why it did not mount.
pub struct Child {
    pass: Pass,
    mounted: Result<Scope, anyhow::Error>,
}

impl Child {
    /// Keeps the child asleep when a later pass leaves it out, instead of
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

/// The window component nearest above `scope`.
fn component<W: slint::ComponentHandle + 'static>(scope: Scope) -> W {
    let found = std::iter::successors(Some(scope), Scope::parent)
        .find_map(|scope| scope.try_kept::<Component, _>(|component| component.0.clone()))
        .unwrap_or_else(|| {
            panic!(
                "root::<{}>() outside a window: this scope is not under one `run` opened",
                std::any::type_name::<W>()
            )
        });

    found
        .downcast_ref::<W>()
        .unwrap_or_else(|| {
            panic!(
                "root::<{}>() but the window is another component",
                std::any::type_name::<W>()
            )
        })
        .clone_strong()
}

fn bind_to_scope<R: Reducer>(scope: Scope, binding: ReducerBinding<R>, apply: impl Fn(&R) + 'static) {
    apply(&binding.peek());
    let reading = binding.clone();
    let reader: Box<dyn Fn()> = Box::new(move || apply(&reading.get()));
    scope.hold_watching(binding.read_by(reader));
}

/// What redraws a tree Slint shows: each property bound to a reducer that
/// moved is set again.
pub(crate) struct Properties;

impl guinea_core::draws::Draws for Properties {
    type Reader = Box<dyn Fn()>;

    fn redraw(&self, readers: guinea_core::draws::Readers<'_, Box<dyn Fn()>>) {
        for set in readers {
            set();
        }
    }
}

/// What a page or a layout binds its globals with.
pub struct BindCx<S> {
    scope: Scope,
    segment: PhantomData<fn() -> S>,
}

impl<S> BindCx<S> {
    /// The window's component, for the globals hanging off it.
    ///
    /// ```
    /// # use guinea_app::app::GuineaHost;
    /// # use guinea_app::app::roots::Root;
    /// # use guinea_slint::{BindCx, Page, Window, page};
    /// # use slint::ComponentHandle;
    /// # slint::slint! {
    /// #     export global ProcessesModel {
    /// #         in-out property <string> title;
    /// #     }
    /// #     export component AppWindow inherits Window {}
    /// # }
    /// # #[derive(Default)]
    /// # struct Processes;
    /// # #[page]
    /// # impl Page for Processes {
    /// #     fn bind(&mut self, cx: &BindCx<Self>) {
    /// let root = cx.root::<AppWindow>();
    /// let model = root.global::<ProcessesModel>();
    /// model.set_title("processes".into());
    /// #     }
    /// # }
    /// # fn main() -> anyhow::Result<()> {
    /// # i_slint_backend_testing::init_no_event_loop();
    /// # let host = GuineaHost::new();
    /// # let window = Root::under(&host.context());
    /// # let component = AppWindow::new()?;
    /// # let _opened = Window::of::<Processes>(component.clone_strong()).open(window.scope())?;
    /// # assert_eq!(component.global::<ProcessesModel>().get_title(), "processes");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Panics with a different type than the window [`run`] was given.
    pub fn root<W: slint::ComponentHandle + 'static>(&self) -> W {
        component::<W>(self.scope)
    }

    /// The navigator over `R` nearest above.
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }

    /// The reducer's binding: state now, and a place to subscribe.
    ///
    /// The feature that answers is the one this segment installed, or the
    /// nearest above that listed `R` in `Exports`. A binding that reaches
    /// nothing panics here, while the segment is being bound.
    pub fn binding<R: Reducer>(&self) -> ReducerBinding<R> {
        binding::<R>(self.scope)
    }

    /// A snapshot of the reducer's state, and what may be asked of the
    /// feature that owns it. Read once: what stays in step is [`Self::bind`].
    pub fn read<R: Reducer>(&self) -> (Rc<R>, Dispatch) {
        let binding = self.binding::<R>();
        (binding.get(), binding.dispatch())
    }

    /// What may be asked of the actor driving `R`.
    pub fn dispatch<R: Reducer>(&self) -> Dispatch {
        self.binding::<R>().dispatch()
    }

    /// Applies the state now, and again after every change, for as long as
    /// this segment's scope lives.
    pub fn bind<R: Reducer>(&self, apply: impl Fn(&R) + 'static) {
        bind_to_scope(self.scope, self.binding::<R>(), apply)
    }

    /// [`Self::bind`], with the root handed back on every call.
    ///
    /// Which is what a binding needs, since the globals it writes to hang off
    /// the root and borrow it - so nothing can capture one. Holding the root
    /// instead is safe: it is a handle, and the window outlives every page.
    pub fn bind_to<R, W>(&self, root: &W, apply: impl Fn(&W, &R) + 'static)
    where
        R: Reducer,
        W: slint::ComponentHandle + 'static,
    {
        let root = root.clone_strong();
        self.bind::<R>(move |state| apply(&root, state))
    }

    /// A list property that reads the state instead of copying it.
    ///
    /// Set once, not on every change: the model answers from the reducer's
    /// own state and converts a row when Slint asks for that row - so a list
    /// of ten thousand costs a screenful, and adding one row costs one.
    ///
    /// ```
    /// # use guinea_app::app::GuineaHost;
    /// # use guinea_app::app::roots::Root;
    /// # use guinea_macros::reducer;
    /// # use guinea_slint::{BindCx, Page, Window, page};
    /// # use slint::{ComponentHandle, Model};
    /// # slint::slint! {
    /// #     export global ProcessesModel {
    /// #         in-out property <[string]> items;
    /// #     }
    /// #     export component AppWindow inherits Window {}
    /// # }
    /// # #[derive(Clone, Default, Debug)]
    /// # struct Processes {
    /// #     items: Vec<String>,
    /// # }
    /// # #[reducer]
    /// # fn processes(this: &mut Processes, items: Vec<String>) {
    /// #     this.items = items;
    /// # }
    /// # #[derive(Default)]
    /// # struct Listing;
    /// # #[page]
    /// # impl Page for Listing {
    /// #     fn install(ctx: &guinea_slint::Install) -> anyhow::Result<Self> {
    /// #         let seeded = Processes { items: vec!["init".into(), "shell".into()] };
    /// #         ctx.state::<Processes>().seed(seeded).plain();
    /// #         Ok(Self)
    /// #     }
    /// #     fn bind(&mut self, cx: &BindCx<Self>) {
    /// #         let root = cx.root::<AppWindow>();
    /// #         let model = root.global::<ProcessesModel>();
    /// model.set_items(cx.rows::<Processes, _>(|state| &state.items));
    /// #     }
    /// # }
    /// # fn main() -> anyhow::Result<()> {
    /// # i_slint_backend_testing::init_no_event_loop();
    /// # let host = GuineaHost::new();
    /// # let window = Root::under(&host.context());
    /// # let component = AppWindow::new()?;
    /// # let _opened = Window::of::<Listing>(component.clone_strong()).open(window.scope())?;
    /// # let items = component.global::<ProcessesModel>().get_items();
    /// # assert_eq!(items.row_count(), 2);
    /// # assert_eq!(items.row_data(1).as_deref(), Some("shell"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn rows<R, T>(&self, select: fn(&R) -> &[T]) -> slint::ModelRc<T::Slint>
    where
        R: Reducer,
        T: ToSlint + 'static,
        T::Slint: Clone + 'static,
    {
        model::Rows::new(self.scope, self.binding::<R>(), select)
    }
}

/// What a layout's [`mount`](Layout::mount) is handed.
pub struct LayoutCx<'a, L> {
    scope: Scope,
    pass: Pass,
    layout: PhantomData<&'a fn() -> L>,
}

impl<L> LayoutCx<'_, L> {
    /// The navigator over `R` nearest above this layout.
    pub fn navigate<R: Routable>(&self) -> Navigator<R> {
        navigator::<R>(self.scope)
    }

    /// See [`BindCx::read`].
    pub fn read<R: Reducer>(&self) -> (Rc<R>, Dispatch) {
        let binding = binding::<R>(self.scope);
        (binding.get(), binding.dispatch())
    }

    /// Mounts `S` - a page or a layout - under this layout, with `params`,
    /// and what is under it.
    ///
    /// Installed the first pass this layout asks for it, in a scope of its
    /// own under the layout's, and the same one every pass after that while
    /// the params are equal. Other params are another child. A child the
    /// layout stops asking for goes when this pass ends - unless it was
    /// [`keep`](Child::keep)s, which puts it to sleep instead.
    pub fn mount<S: Mountable>(&mut self, params: &S::Params) -> Child {
        let mounted = self.pass.mount::<S, _>(params, |cx| S::install(cx, params));
        if let Ok(scope) = &mounted {
            descend(*scope);
        }

        Child {
            pass: self.pass.clone(),
            mounted,
        }
    }

    /// What `nav` says is here, mounted by `mount`: the layout's `match` on
    /// its own level of the route.
    ///
    /// What it mounted is what the navigator asks before it moves on. A child
    /// that fails to mount puts the navigator back where it was, keeps what
    /// stood here before, and leaves why on the navigator's
    /// [`failure`](Navigator::failure) - which the window's global shows.
    pub fn outlet<R: Routable>(&mut self, nav: &Navigator<R>, mount: impl FnOnce(&R, &mut Self) -> Child) {
        let pass = self.pass.clone();
        let _ = pass.outlet(nav, |here| mount(here, self).mounted);
    }
}

#[cfg(test)]
mod tests;
