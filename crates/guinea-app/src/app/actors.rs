//! Actors that belong to the application rather than to a window.

use std::cell::Cell;

use guinea_core::actor::registry::ActorSnapshot;
use guinea_core::actor::{Addr, ManagedActor};
use guinea_core::scope::Scope;

thread_local! {
    /// The scope that lists them: a root of its own, under no window.
    static APP: Cell<Option<Scope>> = const { Cell::new(None) };
}

fn app() -> Scope {
    match APP.get() {
        Some(scope) if scope.is_alive() => scope,
        _ => {
            let scope = Scope::root();
            APP.set(Some(scope));
            scope
        }
    }
}

pub(crate) fn register<A: std::fmt::Debug + ManagedActor>(
    addr: &Addr<A>,
    feature: Option<&'static str>,
) {
    app().hold_actor(addr, feature, None);
}

/// Dropped before teardown checks for leaks: the listing holds an address of
/// every actor in it.
pub(crate) fn forget_all() {
    if let Some(scope) = APP.take() {
        scope.remove();
    }
}

/// Every application-level actor on this thread, by id.
pub fn app_actors() -> Vec<ActorSnapshot> {
    let mut snapshots = APP.get().map(Scope::actors).unwrap_or_default();
    snapshots.sort_by_key(|actor| actor.id);
    snapshots
}

/// The application-level actor `id`, read now; the others are not read.
pub fn app_actor(id: usize) -> Option<ActorSnapshot> {
    APP.get()?.actor(id)
}
