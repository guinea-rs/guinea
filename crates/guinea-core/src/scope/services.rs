use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

use super::Scope;

/// Dependencies owned by one scope, visible to it and its descendants.
#[derive(Default)]
pub(super) struct Services(RefCell<HashMap<TypeId, Provided>>);

struct Provided {
    value: Box<dyn Any>,
    by: &'static str,
}

impl Scope {
    /// Provides `value` as the one `T` visible on this scope's branch, `by` whoever is
    /// named, or hands back who provided a `T` first.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub(crate) fn provide<T: Clone + 'static>(
        &self,
        value: T,
        by: &'static str,
    ) -> Result<(), &'static str> {
        let data = self.alive("providing a dependency");
        if let Some(first) = dependency_provider(*self, TypeId::of::<T>()) {
            return Err(first);
        }

        let mut services = data.services.0.borrow_mut();

        services.insert(
            TypeId::of::<T>(),
            Provided {
                value: Box::new(value),
                by,
            },
        );
        Ok(())
    }

    /// A clone of the `T` provided here or by an ancestor.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub(crate) fn service<T: Clone + 'static>(&self) -> Option<T> {
        self.alive("reading a dependency");
        for owner in std::iter::once(*self).chain(self.ancestors()) {
            let data = owner.alive("reading a dependency");
            let value = data
                .services
                .0
                .borrow()
                .get(&TypeId::of::<T>())
                .and_then(|provided| provided.value.downcast_ref::<T>())
                .cloned();
            if value.is_some() {
                return value;
            }
        }
        None
    }

    /// Whether this scope or an ancestor provides the type `of`.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub(crate) fn provides(&self, of: TypeId) -> bool {
        dependency_provider(*self, of).is_some()
    }
}

pub fn publish_dependency<T: Clone + 'static>(
    scope: Scope,
    value: T,
    by: &'static str,
) -> Result<(), &'static str> {
    scope.provide(value, by)
}

pub fn dependency<T: Clone + 'static>(scope: Scope) -> Option<T> {
    scope.service::<T>()
}

pub fn has_dependency(scope: Scope, id: TypeId) -> bool {
    scope.provides(id)
}

pub fn dependency_provider(scope: Scope, id: TypeId) -> Option<&'static str> {
    scope.alive("reading a dependency provider");
    for owner in std::iter::once(scope).chain(scope.ancestors()) {
        let data = owner.alive("reading a dependency provider");
        if let Some(provided) = data.services.0.borrow().get(&id) {
            return Some(provided.by);
        }
    }
    None
}
