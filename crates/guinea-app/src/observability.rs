//! The installed application, read from outside it: its own scope, the
//! plugins it was built with, and the actors it holds rather than a window.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_core::actor::registry::ActorSnapshot;
use guinea_core::scope::Scope;

use crate::app::registry::Registry;

#[derive(Clone)]
struct Watched {
    scope: Scope,
    registry: Rc<RefCell<Registry>>,
}

thread_local! {
    static WATCHED: RefCell<Option<Watched>> = const { RefCell::new(None) };
}

/// Keeps an application readable from outside for as long as it lives: an
/// installed runtime holds one, and so does a harness. The latest on the
/// thread is the one read.
pub(crate) struct Observed(Scope);

impl Observed {
    pub(crate) fn new(scope: Scope, registry: Rc<RefCell<Registry>>) -> Self {
        WATCHED.with(|slot| *slot.borrow_mut() = Some(Watched { scope, registry }));
        Self(scope)
    }
}

impl Drop for Observed {
    fn drop(&mut self) {
        WATCHED.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_some_and(|watched| watched.scope == self.0) {
                *slot = None;
            }
        });
    }
}

fn watched() -> Option<Watched> {
    WATCHED.with(|slot| slot.borrow().clone())
}

/// The scope of the application on this thread, while it lives.
pub fn app_scope() -> Option<Scope> {
    watched().map(|watched| watched.scope).filter(Scope::is_alive)
}

/// The plugins the application on this thread was built with, by id.
pub fn installed_plugins() -> Vec<&'static str> {
    watched()
        .map(|watched| watched.registry.borrow().plugin_ids())
        .unwrap_or_default()
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
