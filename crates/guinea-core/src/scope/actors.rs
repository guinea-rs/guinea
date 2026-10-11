use std::any::Any;

use crate::actor::Addr;
use crate::actor::snapshot::{ActorSnapshot, Owner};
use crate::actor::shape::Shape;
use crate::actor::traits::ManagedActor;
use crate::observability::changes::{self, Change};

use super::{Scope, Section};
use super::held::Taken;
use super::tree::ScopeData;

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
    snapshot: fn(&dyn Any) -> String,
}

impl HeldActor {
    /// Disposes it, and tells devtools it is gone when they were told it
    /// came.
    pub(super) fn let_go(self, id: usize) {
        (self.dispose)(&*self.addr);

        if self.listing.is_some() {
            changes::changed(|| Change::ActorRemoved { id });
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

fn with_actor<R>(data: &ScopeData, id: usize, with: impl FnOnce(&mut HeldActor) -> R) -> Option<R> {
    match data.held.0.borrow_mut().get_mut(&id)? {
        Taken::Actor(actor) => Some(with(actor)),
        _ => None,
    }
}

fn read_all(scopes: Vec<Scope>) -> Vec<ActorSnapshot> {
    scopes
        .into_iter()
        .filter_map(|scope| scope.data())
        .flat_map(|data| {
            data.held
                .0
                .borrow()
                .iter()
                .filter_map(|(id, taken)| match taken {
                    Taken::Actor(actor) => actor.read(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn read_one(scopes: Vec<Scope>, id: usize) -> Option<ActorSnapshot> {
    scopes
        .into_iter()
        .filter_map(|scope| scope.data())
        .find_map(|data| with_actor(&data, id, |actor| actor.read(id)).flatten())
}

impl Scope {
    /// Takes `addr` on: held until this scope goes, then disposed. What
    /// [`Addr::new`] does with the scope it is made in.
    pub(crate) fn hold<A: 'static>(&self, addr: &Addr<A>) {
        self.alive("making an actor");

        self.take(
            addr.id(),
            Taken::Actor(HeldActor {
                addr: Box::new(addr.clone()),
                dispose: dispose::<A>,
                listing: None,
            }),
        );
    }

    /// The actor `id` this scope holds, if it still does and it is an `A`.
    ///
    /// A clone, taken with the table borrowed only for as long as it takes
    /// to clone: whatever the caller does with it next - send, and so run
    /// handlers that make or dispose actors - finds the table free.
    pub(crate) fn held<A: 'static>(&self, id: usize) -> Option<Addr<A>> {
        let data = self.data()?;
        let held = data.held.0.borrow();

        match held.get(&id)? {
            Taken::Actor(actor) => actor.addr.downcast_ref::<Addr<A>>().cloned(),
            _ => None,
        }
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

impl Section {
    /// Lists `addr` for devtools as this section's, until its scope goes -
    /// and as driving `drives` when it was made to.
    pub(crate) fn list<A: ManagedActor + std::fmt::Debug>(
        &self,
        addr: &Addr<A>,
        drives: Option<&'static str>,
    ) {
        let data = self.scope().alive("listing an actor");
        let id = addr.id();
        let type_name = crate::short_type_name::<A>();
        let owner = Owner {
            scope: Some(self.scope()),
            feature: self.feature(),
            drives,
        };

        let listed = with_actor(&data, id, |held| {
            held.listing = Some(Listing {
                type_name,
                shape: A::SHAPE,
                owner,
                snapshot: snapshot::<A>,
            });
        });
        if listed.is_none() {
            return;
        }

        changes::changed(|| Change::ActorAdded {
            id,
            type_name,
            owner,
        });
    }
}
