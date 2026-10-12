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

    /// Runs `with` on the `T` kept here, and `None` once this scope is gone -
    /// a redraw queued before it went finds nothing to draw.
    ///
    /// Taken out for the call and put back after it, rather than borrowed
    /// across it: what is drawn from it may draw what is under this scope, or
    /// navigate away and remove this scope, after which there is nowhere to
    /// put it back and it goes.
    ///
    /// # Panics
    ///
    /// When this scope is there and keeps no `T`: it was never handed one, or
    /// the call that has it out asks for it again.
    pub fn kept<T: 'static, R>(&self, with: impl FnOnce(&mut T) -> R) -> Option<R> {
        let data = self.data()?;
        let taken = data
            .kept
            .values
            .borrow_mut()
            .get_mut(&TypeId::of::<T>())
            .and_then(Option::take);
        drop(data);

        let Some(taken) = taken else {
            panic!("{self:?} keeps no {}", crate::short_type_name::<T>());
        };
        let mut value = *taken.downcast::<T>().expect("kept under its own type");

        let done = with(&mut value);

        if let Some(data) = self.data()
            && let Some(slot @ None) = data.kept.values.borrow_mut().get_mut(&TypeId::of::<T>())
        {
            *slot = Some(Box::new(value));
        }

        Some(done)
    }
}
