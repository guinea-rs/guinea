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

    /// Whether a `T` is kept here; `false` once this scope is gone.
    pub fn keeps<T: 'static>(&self) -> bool {
        self.data()
            .is_some_and(|data| data.kept.values.borrow().contains_key(&TypeId::of::<T>()))
    }

    /// Runs `with` on the `T` kept here; `None` when this scope keeps no `T`,
    /// and once it is gone - a redraw queued before it went finds nothing to
    /// draw.
    ///
    /// Taken out for the call and put back after it, rather than borrowed
    /// across it: what is drawn from it may draw what is under this scope, or
    /// navigate away and remove this scope, after which there is nowhere to
    /// put it back and it goes.
    ///
    /// ```
    /// # use guinea_core::scope::ScopeTree;
    /// struct Scroll(f32);
    ///
    /// let tree = ScopeTree::new();
    /// let page = tree.child();
    /// assert_eq!(page.kept::<Scroll, _>(|scroll| scroll.0), None);
    ///
    /// page.keep(Scroll(0.0));
    /// page.kept::<Scroll, _>(|scroll| scroll.0 += 120.0);
    /// assert_eq!(page.kept::<Scroll, _>(|scroll| scroll.0), Some(120.0));
    ///
    /// page.remove();
    /// assert_eq!(page.kept::<Scroll, _>(|scroll| scroll.0), None);
    /// ```
    ///
    /// # Panics
    ///
    /// When the call that has the `T` out asks for it again.
    pub fn kept<T: 'static, R>(&self, with: impl FnOnce(&mut T) -> R) -> Option<R> {
        let data = self.data()?;
        let taken = data
            .kept
            .values
            .borrow_mut()
            .get_mut(&TypeId::of::<T>())
            .map(Option::take);
        drop(data);

        let taken = match taken {
            None => return None,
            Some(None) => panic!(
                "{self:?} has its {} out for a call already",
                crate::short_type_name::<T>()
            ),
            Some(Some(taken)) => taken,
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
