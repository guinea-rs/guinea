use crate::scope::{Reducer, Scope};

use super::UiThread;

/// The way back into a reducer, for whoever changes it.
///
/// Handed to an actor as a parameter rather than fetched by name. It names its
/// scope and does not keep it: the scope goes when it is removed, and with it
/// the actor and whatever else was meant to push.
pub struct Push<R: Reducer> {
    scope: Scope,
    reducer: std::marker::PhantomData<R>,
    ui_thread: UiThread,
}

impl<R: Reducer> Clone for Push<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Reducer> Copy for Push<R> {}

impl<R: Reducer> std::fmt::Debug for Push<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push")
            .field("reducer", &std::any::type_name::<R>())
            .field("scope_alive", &self.scope.is_alive())
            .finish()
    }
}

impl<R: Reducer> crate::inspect::Inspect for Push<R> {
    fn inspect(&self, at: crate::inspect::Inspector<'_>) {
        at.unit(&format!("Push<{}>", crate::short_type_name::<R>()));
    }
}

impl<R: Reducer> Push<R> {
    /// The way into `R` in `scope` - for a test that drives an actor without
    /// installing the feature around it.
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            reducer: std::marker::PhantomData,
            ui_thread: std::marker::PhantomData,
        }
    }

    /// Applies an update.
    ///
    /// # Panics
    ///
    /// When its scope is gone: whatever still pushes outlived the page it
    /// was pushing to.
    pub fn send(&self, update: R::Update) {
        crate::trace::mark(|| crate::trace::Point::Push {
            reducer: crate::short_type_name::<R>(),
        });
        self.scope.push::<R>(update);
    }

    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
}
