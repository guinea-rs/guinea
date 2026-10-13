use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

struct Provided {
    value: Box<dyn Any + Send + Sync>,
    by: &'static str,
}

/// What the application provided: one value per type, handed out by clone.
#[derive(Clone, Default)]
pub struct SharedState {
    inner: Arc<RwLock<HashMap<TypeId, Provided>>>,
}

impl SharedState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts `value` in as the one `T`, provided `by` whoever is named, or
    /// hands back who provided a `T` first.
    pub fn provide<T>(&self, value: T, by: &'static str) -> Result<(), &'static str>
    where
        T: Clone + Send + Sync + 'static,
    {
        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);

        if let Some(first) = map.get(&TypeId::of::<T>()) {
            return Err(first.by);
        }

        map.insert(
            TypeId::of::<T>(),
            Provided {
                value: Box::new(value),
                by,
            },
        );
        Ok(())
    }

    /// A clone of the `T` that was provided.
    pub fn get<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        let map = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        map.get(&TypeId::of::<T>())?.value.downcast_ref::<T>().cloned()
    }

    /// Whether anything was provided as the type `id` names.
    pub fn has(&self, id: TypeId) -> bool {
        let map = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        map.contains_key(&id)
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::sync::Arc;

    use super::SharedState;

    #[test]
    fn a_clone_hands_out_what_was_provided_through_the_other() {
        let shared = SharedState::new();
        let clone = shared.clone();

        shared.provide("hello".to_string(), "test").unwrap();

        assert_eq!(clone.get::<String>().as_deref(), Some("hello"));
        assert!(clone.has(TypeId::of::<String>()));
        assert!(!clone.has(TypeId::of::<u32>()));
    }

    #[test]
    fn a_handle_comes_out_as_the_handle_that_went_in() {
        let shared = SharedState::new();
        let service = Arc::new(42usize);
        shared.provide(Arc::clone(&service), "test").unwrap();

        let fetched = shared.get::<Arc<usize>>().expect("provided");
        assert!(Arc::ptr_eq(&fetched, &service));
    }

    #[test]
    fn a_second_value_of_one_type_is_refused_with_who_provided_the_first() {
        let shared = SharedState::new();
        shared.provide(7u32, "first").unwrap();

        assert_eq!(shared.provide(9u32, "second"), Err("first"));
        assert_eq!(shared.get::<u32>(), Some(7));
    }
}
