//! Reading a reducer's state without being a UI hook.
//!
//! A hook is one way to consume state, not the only one: a reconciler wants a
//! snapshot plus a subscription scoped to the component instance, an immediate
//! mode backend polls once a frame, and a retained-property backend wants a
//! synchronous first call followed by pushes. [`ReducerBinding`] is the
//! primitive all three are built on.

use std::cell::Ref;
use std::rc::Rc;

use crate::feature::Dispatch;
use crate::scope::{DropGuard, Reducer, Scope, Slot, Subscription};

/// A handle to one reducer's state and actions inside one scope.
///
/// Names the scope and holds the state cell: reading keeps working after the
/// scope is removed (the last frame of a page being navigated away from still
/// renders), while pushing and subscribing quietly stop.
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
            dispatch: self.dispatch.clone(),
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
        self.dispatch.clone()
    }

    /// Applies an update directly, without an actor in between. A no-op once
    /// the owning scope is gone.
    pub fn push(&self, update: R::Update) {
        self.owner.push::<R>(update);
    }

    /// Calls `f` after every change, until the returned [`Subscription`] is
    /// dropped.
    ///
    /// The caller owns the lifetime, which is what a reconciler needs: the
    /// subscription must end with the component instance, not with the scope,
    /// or unmounted components accumulate as live listeners.
    pub fn on_change(&self, f: impl Fn(&R) + 'static) -> Subscription {
        let state = self.state.clone();
        self.owner.subscribe::<R>(move || {
            let now = state.borrow().clone();
            f(&now)
        })
    }

    /// Like [`Self::on_change`], but the subscription lives as long as the
    /// scope - for backends with no effect system to hang a cleanup on.
    pub fn on_change_owned(&self, f: impl Fn(&R) + 'static) {
        let subscription = self.on_change(f);
        self.owner.own(DropGuard(subscription));
    }

    /// Calls `f` immediately with the current state, then after every change.
    pub fn bind(&self, f: impl Fn(&R) + 'static) -> Subscription {
        f(&self.get());
        self.on_change(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::ScopeTree;
    use std::cell::Cell;

    #[derive(Clone, Default, Debug)]
    struct Counter(u32);

    impl Reducer for Counter {
        type Update = u32;

        fn reduce(&mut self, by: u32) {
            self.0 += by;
        }
    }

    #[test]
    fn a_binding_reads_on_after_its_scope_is_removed() {
        let scope = ScopeTree::new();
        let binding = scope.binding::<Counter>();
        binding.push(1);

        binding.on_change_owned(|_| {});
        scope.remove();

        assert_eq!(binding.peek().0, 1, "the state cell outlives the scope");
        binding.push(1);
        assert_eq!(binding.peek().0, 1, "pushing into a removed scope is a no-op");
    }

    #[test]
    fn subscribing_to_a_removed_scope_hears_nothing() {
        let scope = ScopeTree::new();
        let binding = scope.binding::<Counter>();
        scope.remove();

        let seen = Rc::new(Cell::new(false));
        let recorder = seen.clone();
        let _subscription = binding.on_change(move |_| recorder.set(true));
        binding.push(1);

        assert!(!seen.get());
    }

    #[test]
    fn dropping_a_subscription_after_its_scope_is_harmless() {
        let scope = ScopeTree::new();
        let subscription = scope.binding::<Counter>().on_change(|_| {});
        scope.remove();

        drop(subscription);
    }

    #[test]
    fn on_change_sees_every_push_until_dropped() {
        let scope = ScopeTree::new();
        let binding = scope.binding::<Counter>();

        let seen = Rc::new(Cell::new(0u32));
        let recorder = seen.clone();
        let subscription = binding.on_change(move |counter| recorder.set(counter.0));

        binding.push(2);
        binding.push(3);
        assert_eq!(seen.get(), 5);

        drop(subscription);
        binding.push(4);
        assert_eq!(seen.get(), 5, "no delivery after the subscription is dropped");
    }

    #[test]
    fn bind_calls_back_before_anything_changes() {
        let scope = ScopeTree::new();
        let binding = scope.binding::<Counter>();
        binding.push(7);

        let seen = Rc::new(Cell::new(None));
        let recorder = seen.clone();
        let _subscription = binding.bind(move |counter| recorder.set(Some(counter.0)));

        assert_eq!(seen.get(), Some(7));
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
