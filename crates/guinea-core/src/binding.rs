//! Reading a reducer's state without being a UI hook.
//!
//! A hook is one way to consume state, not the only one: a reconciler wants a
//! snapshot plus a reader scoped to the component instance, an immediate mode
//! backend reads once a frame, and a retained-property backend reads once and
//! is redrawn after. [`ReducerBinding`] is the primitive all three are built
//! on; what redraws a reader is the tree's [`Draws`](crate::draws::Draws).

use std::cell::Ref;
use std::rc::Rc;

use crate::feature::Dispatch;
use crate::scope::{Reducer, Scope, Slot, Subscription};

/// A handle to one reducer's state and actions inside one scope.
///
/// Names the scope and holds the state cell: reading keeps working after the
/// scope is removed (the last frame of a page being navigated away from still
/// renders). Pushing or subscribing after that is a mistake, and says so.
pub struct ReducerBinding<R: Reducer> {
    owner: Scope,
    state: Rc<Slot<R>>,
    dispatch: Dispatch,
}

impl<R: Reducer> Clone for ReducerBinding<R> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner,
            state: self.state.clone(),
            dispatch: self.dispatch,
        }
    }
}

impl<R: Reducer> ReducerBinding<R> {
    pub(crate) fn new(scope: Scope) -> Self {
        Self {
            owner: scope,
            state: scope.state::<R>(),
            dispatch: Dispatch::owning::<R>(scope),
        }
    }

    /// Borrows the state in place, for as long as nothing changes it.
    pub fn peek(&self) -> Ref<'_, R> {
        Ref::map(self.state.borrow(), |state| &**state)
    }

    /// The state as it is now, shared rather than copied. A change made while
    /// it is held goes to a copy, so what was read stays as it was.
    pub fn get(&self) -> Rc<R> {
        self.state.borrow().clone()
    }

    /// What may be asked of the actors in the scope that owns this reducer.
    pub fn dispatch(&self) -> Dispatch {
        self.dispatch
    }

    /// Applies an update directly, without an actor in between.
    ///
    /// # Panics
    ///
    /// When the owning scope is gone.
    pub fn push(&self, update: R::Update) {
        self.owner.push::<R>(update);
    }

    /// Registers `reader` as something drawn from this reducer: after a turn
    /// that moved it, the tree's [`Draws`](crate::draws::Draws) is handed
    /// `reader` to redraw, until the returned [`Subscription`] is dropped.
    ///
    /// The caller owns the lifetime: a reconciler ends it with the component
    /// instance, a backend with nothing to hang a cleanup on hands it to the
    /// scope's [`hold_watching`](Scope::hold_watching).
    ///
    /// # Panics
    ///
    /// When the owning scope is gone, and when nothing draws its tree with
    /// readers of this type.
    pub fn read_by<D: 'static>(&self, reader: D) -> Subscription {
        self.owner.read_by::<R, D>(reader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::ScopeTree;

    #[derive(Clone, Default, Debug)]
    struct Counter(u32);

    #[guinea_macros::reducer]
    fn counter(this: &mut Counter, by: u32) {
        this.0 += by;
    }

    #[test]
    fn a_binding_reads_on_after_its_scope_is_removed() {
        let tree = ScopeTree::new();
        tree.drawn_by(crate::draws::Calls);
        let scope = tree.child();
        let binding = scope.binding::<Counter>();
        binding.push(1);

        scope.hold_watching(binding.read_by(Box::new(|| {}) as Box<dyn Fn()>));
        scope.remove();

        assert_eq!(binding.peek().0, 1, "the state cell outlives the scope");
        assert_eq!(binding.get().0, 1);
    }

    #[test]
    fn dropping_a_reader_after_its_scope_is_harmless() {
        let tree = ScopeTree::new();
        tree.drawn_by(crate::draws::Calls);
        let scope = tree.child();
        let reading = scope.binding::<Counter>().read_by(Box::new(|| {}) as Box<dyn Fn()>);
        scope.remove();

        drop(reading);
    }

    #[test]
    fn a_read_is_shared_and_a_change_while_it_is_held_goes_to_a_copy() {
        let scope = ScopeTree::new();
        let binding = scope.binding::<Counter>();
        binding.push(1);

        let first = binding.get();
        let again = binding.get();
        assert!(Rc::ptr_eq(&first, &again), "reading twice copies nothing");

        binding.push(2);
        assert_eq!(first.0, 1, "what was read stays as it was");
        assert_eq!(binding.get().0, 3);

        drop((first, again));
        let before = Rc::as_ptr(&binding.get());
        binding.push(4);
        assert_eq!(Rc::as_ptr(&binding.get()), before, "nobody held it, so no copy");
    }
}
