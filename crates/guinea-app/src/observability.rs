//! An installed application, read from outside it: the plugins it was built
//! with, and the actors it holds rather than a window.

use guinea_core::actor::snapshot::ActorSnapshot;
use guinea_core::scope::Scope;

use crate::app::registry::{Backend, Plugins};

/// The plugins the application whose scope is `app` was built with, by id;
/// none once it is gone.
///
/// # Panics
///
/// When `app` is not an application's scope.
pub fn installed_plugins(app: Scope) -> Vec<&'static str> {
    app.kept::<Plugins, _>(|plugins| plugins.ids())
        .unwrap_or_default()
}

/// What the application whose scope is `app` draws with - `"winui"`,
/// `"egui"` - as the backend that started it says; `None` for one no
/// backend started, as under a test.
pub fn backend(app: Scope) -> Option<&'static str> {
    app.kept::<Backend, _>(|backend| backend.0).flatten()
}

/// Every actor the application whose scope is `app` holds itself, by id.
pub fn app_actors(app: Scope) -> Vec<ActorSnapshot> {
    let mut snapshots = app.actors_here();
    snapshots.sort_by_key(|actor| actor.id);
    snapshots
}

/// The actor `id` the application whose scope is `app` holds, read now; the
/// others are not read.
pub fn app_actor(app: Scope, id: usize) -> Option<ActorSnapshot> {
    app.actor_here(id)
}
