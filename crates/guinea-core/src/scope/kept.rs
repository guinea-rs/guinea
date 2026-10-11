use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

use super::Scope;

/// What a scope keeps for whoever draws it, one value of each type.
#[derive(Default)]
pub(super) struct Kept {
    values: RefCell<HashMap<TypeId, Option<Box<dyn Any>>>>,
}

impl Scope {
    /// Keeps `value` for as long as this scope is there, in place of any `T`
    /// kept here before.
    pub fn keep<T: 'static>(&self, value: T) {
        let data = self.alive("keeping a value");
        let replaced = data
            .kept
            .values
            .borrow_mut()
            .insert(TypeId::of::<T>(), Some(Box::new(value)));
        drop(replaced);
    }

    /// Runs `with` on the `T` kept here - or, when none is, on a default that
    /// lasts the call.
    ///
    /// Taken out for the call and put back after it, rather than borrowed
    /// across it: what is drawn from it may draw what is under this scope, or
    /// navigate away and remove this scope, after which there is nowhere to
    /// put it back and it goes.
    pub fn kept<T: Default + 'static, R>(&self, with: impl FnOnce(&mut T) -> R) -> R {
        let taken = self.data().and_then(|data| {
            let mut values = data.kept.values.borrow_mut();
            values.get_mut(&TypeId::of::<T>()).and_then(Option::take)
        });
        let mut value = taken
            .and_then(|value| value.downcast::<T>().ok())
            .map_or_else(T::default, |value| *value);

        let done = with(&mut value);

        if let Some(data) = self.data()
            && let Some(slot @ None) = data.kept.values.borrow_mut().get_mut(&TypeId::of::<T>())
        {
            *slot = Some(Box::new(value));
        }

        done
    }
}
