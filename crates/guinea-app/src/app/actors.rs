//! The installed application's own scope, and the actors it holds rather than
//! a window.

use guinea_core::actor::registry::ActorSnapshot;
use guinea_core::scope::Scope;

/// The scope of the application installed on this thread, while it is.
pub fn app_scope() -> Option<Scope> {
    super::runtime::installed_scope().filter(Scope::is_alive)
}

/// Every application-level actor on this thread, by id.
pub fn app_actors() -> Vec<ActorSnapshot> {
    let mut snapshots = app_scope()
        .map(|scope| scope.actors_here())
        .unwrap_or_default();
    snapshots.sort_by_key(|actor| actor.id);
    snapshots
}

/// The application-level actor `id`, read now; the others are not read.
pub fn app_actor(id: usize) -> Option<ActorSnapshot> {
    app_scope()?.actor_here(id)
}
