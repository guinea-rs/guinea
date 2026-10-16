//! What a scope mounts under it while it draws.

use std::any::{Any, TypeId};
use std::rc::Rc;

use guinea_app::feature::FeatureInitContext;
use guinea_core::actor::shape::Declared;
use guinea_core::__private::ReducerBinding;
use guinea_core::feature::Dispatch;
use guinea_core::__private::observer;
use guinea_core::observability::Event;
use guinea_core::scope::{Reducer, Scope};

use crate::navigator::{Navigator, Routable};

/// One drawing of a parent: every child it mounts during the pass is looked
/// up by its type and its params, made the first time, and kept from one
/// pass to the next for as long as the parent mounts it again.
///
/// A child is the same child only for the same type and equal params. Other
/// params are another child in a scope of its own, and the one left out goes
/// when the pass ends - asleep instead, when it was [`keep`](Self::keep)s.
#[derive(Clone)]
pub struct Pass {
    parent: Scope,
    number: u64,
}

/// What a parent has mounted, kept in the parent's scope.
#[derive(Default)]
struct Mounts {
    children: Vec<Child>,
    passes: u64,
}

/// What an outlet draws after [`Pass::outlet`].
#[derive(Debug)]
pub enum Outlet {
    /// What the navigator is at, mounted.
    Here(Scope),
    /// What stood before, kept while what the navigator moved to failed.
    Held(Scope),
    /// Why what the navigator is at did not mount, with nothing before it.
    Failed(anyhow::Error),
    /// The navigator is somewhere this level is not part of.
    Nowhere,
}

/// The type a scope was mounted as, kept in the scope.
struct Named(&'static str);

struct Child {
    kind: TypeId,
    params: Box<dyn Any>,
    scope: Scope,
    drawn: u64,
    keep: bool,
}

impl Mounts {
    fn find<P: PartialEq + 'static>(&self, kind: TypeId, params: &P) -> Option<usize> {
        self.children.iter().position(|child| {
            child.kind == kind && child.params.downcast_ref::<P>().is_some_and(|mine| mine == params)
        })
    }
}

impl Pass {
    /// Starts a drawing of `parent`.
    pub fn begin(parent: Scope) -> Self {
        if !parent.keeps::<Mounts>() {
            parent.keep(Mounts::default());
        }

        let number = parent
            .kept::<Mounts, _>(|mounts| {
                mounts.passes += 1;
                mounts.passes
            })
            .unwrap_or_default();

        Self { parent, number }
    }

    /// The scope of the `S` mounted with `params`: the one from an earlier
    /// pass, or a new child of the parent with `install` run in it.
    ///
    /// A child that fails to install is removed before the error comes back,
    /// and the others stand.
    pub fn mount<S: 'static, P: PartialEq + Clone + 'static>(
        &self,
        params: &P,
        install: impl FnOnce(&FeatureInitContext) -> anyhow::Result<()>,
    ) -> anyhow::Result<Scope> {
        let kind = TypeId::of::<S>();
        let number = self.number;

        let found = self
            .parent
            .kept::<Mounts, _>(|mounts| {
                let at = mounts.find(kind, params)?;
                let child = &mut mounts.children[at];
                child.drawn = number;
                Some(child.scope)
            })
            .flatten();

        if let Some(scope) = found {
            if !scope.is_awake() {
                scope.wake();
            }
            return Ok(scope);
        }

        let scope = self.parent.child();
        scope.keep(Named(std::any::type_name::<S>()));
        let cx = FeatureInitContext {
            scope_cx: guinea_app::feature::ScopeContext::new(scope),
            cursor: 0,
        };
        if let Err(error) = install(&cx) {
            scope.remove();
            let name = crate::observability::short(std::any::type_name::<S>());
            return Err(error.context(format!("{name} did not mount")));
        }
        told_mounted(scope);

        self.parent.kept::<Mounts, _>(|mounts| {
            mounts.children.push(Child {
                kind,
                params: Box::new(params.clone()),
                scope,
                drawn: number,
                keep: false,
            })
        });
        Ok(scope)
    }

    /// Keeps `child` asleep when a later pass leaves it out, instead of
    /// removing it.
    pub fn keep(&self, child: Scope) {
        self.parent.kept::<Mounts, _>(|mounts| {
            if let Some(mounted) = mounts.children.iter_mut().find(|mounted| mounted.scope == child) {
                mounted.keep = true;
            }
        });
    }

    /// Lets `child` stand through this pass without mounting it: what was
    /// drawn before, kept while what was to replace it failed.
    pub fn hold(&self, child: Scope) {
        let number = self.number;
        self.parent.kept::<Mounts, _>(|mounts| {
            if let Some(held) = mounts.children.iter_mut().find(|held| held.scope == child) {
                held.drawn = number;
            }
        });
    }

    /// What `nav` says is here at the parent's level, mounted by `draw`.
    ///
    /// What it mounted is recorded on the navigator, for the guards to ask
    /// before it moves on. One that fails to mount puts the navigator back
    /// where it was, with why as its [`failure`](Navigator::failure), and
    /// what stood here before stands through this pass.
    pub fn outlet<R: Routable>(
        &self,
        nav: &Navigator<R>,
        draw: impl FnOnce(&R) -> anyhow::Result<Scope>,
    ) -> Outlet {
        let Some(here) = nav.current() else {
            return Outlet::Nowhere;
        };

        match draw(&here) {
            Ok(child) => {
                nav.rendered(self.parent, child, &here);
                Outlet::Here(child)
            }
            Err(error) => {
                let standing = nav.drawn(self.parent);
                match standing {
                    Some(standing) => {
                        nav.failed(error);
                        self.hold(standing);
                        Outlet::Held(standing)
                    }
                    None => {
                        nav.failed(anyhow::anyhow!("{error:#}"));
                        Outlet::Failed(error)
                    }
                }
            }
        }
    }

    /// Ends the drawing: what it did not mount goes, or sleeps when it was
    /// kept.
    pub fn end(self) {
        let number = self.number;
        let Some(left) = self.parent.kept::<Mounts, _>(|mounts| {
            let mut left = Vec::new();
            mounts.children.retain(|child| {
                if child.drawn == number {
                    return true;
                }
                left.push((child.scope, child.keep));
                child.keep
            });
            left
        }) else {
            return;
        };

        for (scope, keep) in left {
            match keep {
                true if scope.is_awake() => scope.sleep(),
                true => {}
                false => scope.remove(),
            }
        }
    }
}

/// What `parent` mounted and still holds, in the order it first mounted them,
/// each with its type's name.
pub fn mounted(parent: Scope) -> Vec<(Scope, &'static str)> {
    if !parent.keeps::<Mounts>() {
        return Vec::new();
    }
    let children: Vec<Scope> = parent
        .kept::<Mounts, _>(|mounts| mounts.children.iter().map(|child| child.scope).collect())
        .unwrap_or_default();
    children
        .into_iter()
        .filter_map(|child| Some((child, name(child)?)))
        .collect()
}

/// The navigator over `R` nearest above `scope`, or a panic naming what was
/// looked for: a view that navigates where none was opened is a wiring
/// mistake.
pub fn navigator<R: Routable>(scope: Scope) -> Navigator<R> {
    Navigator::find(scope).unwrap_or_else(|| {
        panic!(
            "no navigator over `{}` above `{}`: open one where the window starts, \
             or in a layout above",
            crate::observability::short(std::any::type_name::<R>()),
            name(scope).map(crate::observability::short).unwrap_or("this view"),
        )
    })
}

/// A page or a layout being drawn, as far as reading goes - whatever backend
/// draws it.
///
/// What lets a plugin offer one shortcut on every backend. A segment reads
/// `R` through it and is drawn again when `R` changes: by subscribing, or by
/// drawing every frame anyway. A backend that builds its view once and binds
/// it is not one of these - it binds.
///
/// What is read is found from the segment's scope by [`binding`]; a backend
/// says which scope that is, and overrides [`read`](Self::read) only when
/// reading also subscribes.
pub trait Reads {
    /// The scope of the segment being drawn.
    #[doc(hidden)]
    fn __scope(&self) -> Scope;

    /// `R` as it is now, and what may be asked of the feature that owns it.
    fn read<R>(&mut self) -> (Rc<R>, Dispatch)
    where
        R: Reducer + PartialEq,
    {
        let binding = binding::<R>(self.__scope());
        (binding.get(), binding.dispatch())
    }

    /// What may be asked of the feature that owns `R`, without reading it:
    /// the segment is not drawn again when `R` changes.
    fn dispatch<R>(&self) -> Dispatch
    where
        R: Reducer,
    {
        binding::<R>(self.__scope()).dispatch()
    }
}

/// The scope that owns `R`, as seen from `scope`, and a binding to its state.
///
/// The scope itself counts whatever it claimed; one above counts only what it
/// *exported*. A miss is a setup bug rather than a race: what a scope claims
/// is recorded while it installs, before its first render. So it panics,
/// naming what `scope` was mounted under.
pub fn binding<R: Reducer>(scope: Scope) -> ReducerBinding<R> {
    let owner = scope.owner_of::<R>().unwrap_or_else(|| {
        if !scope.is_alive() {
            panic!(
                "reading {} in {scope:?}, which was removed: whatever mounted it let go of it \
                 while it still renders",
                std::any::type_name::<R>()
            );
        }

        panic!("{}", unreached::<R>(scope))
    });
    owner.binding::<R>()
}

fn unreached<R: 'static>(scope: Scope) -> String {
    let short = crate::observability::short;
    let reducer = short(std::any::type_name::<R>());
    let walked: Vec<Scope> = std::iter::successors(Some(scope), Scope::parent).collect();
    let named = |scope: Scope| name(scope).map(short);
    let names: Vec<&str> = walked.iter().filter_map(|scope| named(*scope)).collect();

    let mut said = format!(
        "`{}` reads `{reducer}`, but nothing reaches it:\n  {}",
        names.first().copied().unwrap_or("this view"),
        names.join(" <- ")
    );

    let kept: Vec<&str> = walked[1..]
        .iter()
        .filter(|scope| scope.claims::<R>())
        .map(|scope| named(*scope).unwrap_or("the window"))
        .collect();
    if kept.is_empty() {
        said.push_str(&format!(
            "\n  nothing above it installs the feature that owns `{reducer}` - it installs \
             elsewhere, or not at all"
        ));
    } else {
        said.push_str(&format!(
            "\n  `{reducer}` is claimed by {}, which does not list it in `Exports`",
            kept.join(", ")
        ));
    }
    said
}

/// Names `scope` as an `S` that no parent's pass mounted: what a test
/// installs on its own.
pub fn root<S: 'static>(scope: Scope) {
    scope.keep(Named(std::any::type_name::<S>()));
    told_mounted(scope);
}

/// Tells the tool watching that `scope` is mounted, as what and where it was
/// written.
fn told_mounted(scope: Scope) {
    let Some(name) = name(scope) else {
        return;
    };
    observer::tell(scope, || Event::Mounted {
        scope: scope.id(),
        name,
        written: written_at(scope),
    });
}

/// What a tool that starts watching is told of what `scope` was mounted as,
/// when it was.
pub(crate) fn describe(scope: Scope, events: &mut Vec<Event>) {
    if let Some(name) = name(scope) {
        events.push(Event::Mounted {
            scope: scope.id(),
            name,
            written: written_at(scope),
        });
    }
}

/// Where what is mounted into `scope` was written, for devtools to link to.
pub fn written(scope: Scope, declared: Option<Declared>) {
    if let Some(declared) = declared {
        scope.keep(Written(declared));
    }
}

struct Written(Declared);

/// Where what is mounted into `scope` was written, if it said.
pub(crate) fn written_at(scope: Scope) -> Option<Declared> {
    scope.keeps::<Written>().then(|| scope.kept::<Written, _>(|written| written.0)).flatten()
}

/// The name of the type `scope` was mounted as.
pub fn name(scope: Scope) -> Option<&'static str> {
    scope.keeps::<Named>().then(|| scope.kept::<Named, _>(|named| named.0)).flatten()
}
