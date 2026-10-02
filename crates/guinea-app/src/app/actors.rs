//! The application's own scope on this thread, and the actors it holds rather
//! than a window.
//!
//! Named here, not kept: the [`AppHost`](super::AppHost) owns the scope, and
//! this goes dead with it.

use std::cell::Cell;

use guinea_core::actor::registry::ActorSnapshot;
use guinea_core::scope::Scope;

thread_local! {
    static APP: Cell<Option<Scope>> = const { Cell::new(None) };
}

/// Makes `scope` the application's scope on this thread.
pub(crate) fn set_app_scope(scope: Scope) {
    APP.set(Some(scope));
}

/// The scope of the application installed on this thread, while it is.
pub fn app_scope() -> Option<Scope> {
    APP.get().filter(Scope::is_alive)
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
