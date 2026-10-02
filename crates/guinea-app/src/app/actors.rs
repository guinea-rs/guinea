//! Actors that belong to the application rather than to a window.

use std::rc::Rc;

use guinea_core::actor::{Addr, ManagedActor};
use guinea_core::actor::registry::{ActorSnapshot, DebugRegistry, Owner};

thread_local! {
    static APP: Rc<DebugRegistry> = Rc::new(DebugRegistry::new());
}

pub(crate) fn register<A: std::fmt::Debug + ManagedActor>(
    addr: &Addr<A>,
    feature: Option<&'static str>,
) {
    let owner = Owner {
        feature,
        ..Owner::default()
    };
    APP.with(|registry| registry.register_owned(addr, owner));
}

/// Dropped before teardown checks for leaks: the registry holds an address
/// of every actor it lists.
pub(crate) fn forget_all() {
    APP.with(|registry| registry.clear());
}

/// Every application-level actor on this thread, by id.
pub fn app_actors() -> Vec<ActorSnapshot> {
    let mut snapshots = APP.with(|registry| registry.snapshots());
    snapshots.sort_by_key(|actor| actor.id);
    snapshots
}

/// The application-level actor `id`, read now; the others are not read.
pub fn app_actor(id: usize) -> Option<ActorSnapshot> {
    APP.with(|registry| registry.snapshot_of(id))
}
