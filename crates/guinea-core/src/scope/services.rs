use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

use super::Scope;

/// What a tree's root holds as its services: one value per type, handed out
/// by clone. They go with the root, after everything it held.
#[derive(Default)]
pub(super) struct Services(RefCell<HashMap<TypeId, Provided>>);

struct Provided {
    value: Box<dyn Any>,
    by: &'static str,
}

impl Scope {
    /// Provides `value` as the one `T` of this scope's tree, `by` whoever is
    /// named, or hands back who provided a `T` first.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub fn provide<T: Clone + 'static>(&self, value: T, by: &'static str) -> Result<(), &'static str> {
        let root = self.root_alive("providing a service");
        let mut services = root.services.0.borrow_mut();

        if let Some(first) = services.get(&TypeId::of::<T>()) {
            return Err(first.by);
        }

        services.insert(
            TypeId::of::<T>(),
            Provided {
                value: Box::new(value),
                by,
            },
        );
        Ok(())
    }

    /// A clone of the `T` provided to this scope's tree.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub fn service<T: Clone + 'static>(&self) -> Option<T> {
        let root = self.root_alive("reading a service");
        let services = root.services.0.borrow();

        services.get(&TypeId::of::<T>())?.value.downcast_ref::<T>().cloned()
    }

    /// Whether anything was provided to this scope's tree as the type `of`.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub fn provides(&self, of: TypeId) -> bool {
        let root = self.root_alive("reading a service");
        let provides = root.services.0.borrow().contains_key(&of);
        provides
    }
}
