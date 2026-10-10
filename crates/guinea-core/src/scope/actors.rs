use std::any::Any;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use crate::actor::Addr;
use crate::actor::registry::{ActorSnapshot, Owner};
use crate::actor::shape::Shape;
use crate::actor::traits::ManagedActor;
use crate::observability::changes::{self, Change};

use super::{Scope, ScopeData, Teardown};

/// The actors a scope owns, by id: ids grow as actors are made, so this is
/// also the order they were.
pub(super) type Held = BTreeMap<usize, HeldActor>;

/// An actor its scope owns: the only strong hold on it once whoever made it
/// lets go of its address.
pub(super) struct HeldActor {
    addr: Box<dyn Any>,
    dispose: fn(&dyn Any),
    listing: Option<Listing>,
}

/// What devtools are told about an actor a feature made.
struct Listing {
    type_name: &'static str,
    shape: Shape,
    owner: Owner,
    root: Option<u64>,
    snapshot: fn(&dyn Any) -> String,
}

/// An actor's scope going: the actor is taken out of it, disposed, and no
/// longer listed.
struct Leaving {
    data: Weak<ScopeData>,
    id: usize,
}

impl Teardown for Leaving {
    fn teardown(self) {
        let Some(data) = self.data.upgrade() else { return };
        let gone = data.actors.borrow_mut().remove(&self.id);
        drop(data);

        if let Some(gone) = gone {
            (gone.dispose)(&*gone.addr);
            gone.unlisted(self.id);
        }
    }
}

impl HeldActor {
    /// Tells devtools it is gone, when they were told it came.
    fn unlisted(self, id: usize) {
        if let Some(listing) = &self.listing {
            changes::changed(|| Change::ActorRemoved {
                root: listing.root,
                id,
            });
        }
    }

    fn read(&self, id: usize) -> Option<ActorSnapshot> {
        let listing = self.listing.as_ref()?;
        Some(ActorSnapshot {
            id,
            type_name: listing.type_name,
            shape: listing.shape,
            owner: listing.owner,
            state: (listing.snapshot)(&*self.addr),
        })
    }
}

fn dispose<A: 'static>(addr: &dyn Any) {
    if let Some(addr) = addr.downcast_ref::<Addr<A>>() {
        addr.dispose();
    }
}

fn snapshot<A: std::fmt::Debug + 'static>(addr: &dyn Any) -> String {
    addr.downcast_ref::<Addr<A>>()
        .map(Addr::debug_snapshot)
        .unwrap_or_default()
}

fn read_all(scopes: Vec<Scope>) -> Vec<ActorSnapshot> {
    scopes
        .into_iter()
        .filter_map(|scope| scope.data())
        .flat_map(|data| {
            data.actors
                .borrow()
                .iter()
                .filter_map(|(id, actor)| actor.read(*id))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn read_one(scopes: Vec<Scope>, id: usize) -> Option<ActorSnapshot> {
    scopes
        .into_iter()
        .filter_map(|scope| scope.data())
        .find_map(|data| data.actors.borrow().get(&id)?.read(id))
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

    /// Takes `addr` on: held until this scope goes, then disposed. What
    /// [`Addr::new`] does with the scope it is made in.
    pub(crate) fn hold<A: 'static>(&self, addr: &Addr<A>) {
        let Some(data) = self.data() else {
            addr.dispose();
            return;
        };

        data.actors.borrow_mut().insert(
            addr.id(),
            HeldActor {
                addr: Box::new(addr.clone()),
                dispose: dispose::<A>,
                listing: None,
            },
        );

        let id = addr.id();
        let data = Rc::downgrade(&data);
        self.own(Leaving { data, id });
    }

    /// The actor `id` this scope holds, if it still does and it is an `A`.
    ///
    /// A clone, taken with the table borrowed only for as long as it takes
    /// to clone: whatever the caller does with it next - send, and so run
    /// handlers that make or dispose actors - finds the table free.
    pub(crate) fn held<A: 'static>(&self, id: usize) -> Option<Addr<A>> {
        let data = self.data()?;
        let actors = data.actors.borrow();
        actors.get(&id)?.addr.downcast_ref::<Addr<A>>().cloned()
    }

    /// Lets go of the actor `id`, disposed before this scope went.
    pub(crate) fn release(&self, id: usize) {
        let Some(data) = self.data() else { return };
        let gone = data.actors.borrow_mut().remove(&id);
        if let Some(gone) = gone {
            gone.unlisted(id);
        }
    }

    /// Lists `addr` for devtools, under the feature installing now, until
    /// this scope goes - and as driving `drives` when it was made to.
    pub(crate) fn list<A: ManagedActor + std::fmt::Debug>(
        &self,
        addr: &Addr<A>,
        drives: Option<&'static str>,
    ) {
        let Some(data) = self.data() else { return };
        let id = addr.id();
        let type_name = crate::actor::short_type_name::<A>();
        let owner = Owner {
            scope: Some(self.key()),
            feature: self.current_feature(),
            drives,
        };
        let root = self.window();

        let mut actors = data.actors.borrow_mut();
        let Some(held) = actors.get_mut(&id) else { return };
        held.listing = Some(Listing {
            type_name,
            shape: A::SHAPE,
            owner,
            root,
            snapshot: snapshot::<A>,
        });
        drop(actors);

        changes::changed(|| Change::ActorAdded {
            root,
            id,
            type_name,
            owner,
        });
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
