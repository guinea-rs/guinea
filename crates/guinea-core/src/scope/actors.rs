use crate::actor::Addr;
use crate::actor::registry::{ActorSnapshot, Owner};
use crate::actor::traits::ManagedActor;

use super::{Scope, Teardown};

pub(super) struct HeldActor {
    id: usize,
    type_name: &'static str,
    shape: crate::actor::shape::Shape,
    owner: Owner,
    snapshot: Box<dyn Fn() -> String>,
}

impl HeldActor {
    fn read(&self) -> ActorSnapshot {
        ActorSnapshot {
            id: self.id,
            type_name: self.type_name,
            shape: self.shape,
            owner: self.owner,
            state: (self.snapshot)(),
        }
    }
}

fn read_all(scopes: Vec<Scope>) -> Vec<ActorSnapshot> {
    scopes
        .into_iter()
        .filter_map(|scope| scope.data())
        .flat_map(|data| data.actors.borrow().iter().map(HeldActor::read).collect::<Vec<_>>())
        .collect()
}

fn read_one(scopes: Vec<Scope>, id: usize) -> Option<ActorSnapshot> {
    scopes.into_iter().filter_map(|scope| scope.data()).find_map(|data| {
        data.actors
            .borrow()
            .iter()
            .find(|actor| actor.id == id)
            .map(HeldActor::read)
    })
}

/// Tells devtools an actor is no longer listed, when its scope goes.
struct Unlisted {
    root: Option<u64>,
    id: usize,
}

impl Teardown for Unlisted {
    fn teardown(self) {
        crate::devtools::changed(|| crate::devtools::Change::ActorRemoved {
            root: self.root,
            id: self.id,
        });
    }
}

impl Scope {
    /// Says this scope is the root of window `id`, as `RootId::get` numbers
    /// it: what devtools are told the actors under it belong to.
    pub fn set_window(&self, id: u64) {
        if let Some(data) = self.data() {
            data.window.set(Some(id));
        }
    }

    /// The window this scope is under, if it is under one.
    pub fn window(&self) -> Option<u64> {
        std::iter::successors(Some(*self), Scope::parent)
            .find_map(|scope| scope.data()?.window.get())
    }

    /// Lists `addr` among the actors this scope holds, for devtools, until
    /// the scope is removed - `feature` created it, and it drives `drives`
    /// when it was made to.
    ///
    /// Only the listing: what ends the actor is still whoever owns it, which
    /// for an actor of a segment is [`own`](Self::own) on this same scope.
    pub fn hold_actor<A: ManagedActor + std::fmt::Debug>(
        &self,
        addr: &Addr<A>,
        feature: Option<&'static str>,
        drives: Option<&'static str>,
    ) {
        let Some(data) = self.data() else { return };
        let id = addr.id();
        let type_name = crate::actor::short_type_name::<A>();
        let owner = Owner {
            scope: Some(self.key()),
            feature,
            drives,
        };

        let held = addr.clone();
        data.actors.borrow_mut().push(HeldActor {
            id,
            type_name,
            shape: A::SHAPE,
            owner,
            snapshot: Box::new(move || held.debug_snapshot()),
        });
        drop(data);

        let root = self.window();
        crate::devtools::changed(|| crate::devtools::Change::ActorAdded {
            root,
            id,
            type_name,
            owner,
        });
        self.own(Unlisted { root, id });
    }

    /// The actors this scope and every scope under it hold, read now.
    pub fn actors(&self) -> Vec<ActorSnapshot> {
        read_all(self.subtree())
    }

    /// The actor `id`, if this scope or one under it holds it, read now; the
    /// others are not read.
    pub fn actor(&self, id: usize) -> Option<ActorSnapshot> {
        read_one(self.subtree(), id)
    }

    /// The actors this scope holds itself, read now - not those of the
    /// scopes under it.
    pub fn actors_here(&self) -> Vec<ActorSnapshot> {
        read_all(vec![*self])
    }

    /// The actor `id`, if this scope holds it itself, read now.
    pub fn actor_here(&self, id: usize) -> Option<ActorSnapshot> {
        read_one(vec![*self], id)
    }
}
