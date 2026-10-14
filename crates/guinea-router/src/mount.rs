//! What a scope mounts under it while it draws.

use std::any::{Any, TypeId};

use guinea_app::feature::FeatureInitContext;
use guinea_core::scope::Scope;

/// One drawing of a parent: every child it mounts during the pass is looked
/// up by its type and its params, made the first time, and kept from one
/// pass to the next for as long as the parent mounts it again.
///
/// A child is the same child only for the same type and equal params. Other
/// params are another child in a scope of its own, and the one left out goes
/// when the pass ends - asleep instead, when it was [`keep`](Self::keep)s.
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
        let cx = FeatureInitContext {
            scope_cx: guinea_app::feature::ScopeContext::new(scope),
            cursor: 0,
        };
        if let Err(error) = install(&cx) {
            scope.remove();
            return Err(error);
        }

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
