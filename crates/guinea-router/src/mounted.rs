//! A segment's own state, for an immediate-mode backend.
//!
//! A retained backend keeps a page's state in the component it mounted. An
//! immediate one has nowhere: its view is drawn once and dropped. So the
//! state is kept here, by the scope it is mounted in and what it is, and dies
//! with that scope.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

use guinea_core::scope::Scope;

thread_local! {
    static MOUNTED: RefCell<HashMap<(usize, TypeId), Option<Box<dyn Any>>>> =
        RefCell::new(HashMap::new());
}

/// Holds `state` for as long as `scope` lives.
pub fn keep<S: 'static>(scope: &Scope, state: S) {
    let at = (scope.key(), TypeId::of::<S>());

    MOUNTED.with(|mounted| mounted.borrow_mut().insert(at, Some(Box::new(state))));
    scope.own(Forget(at));
}

struct Forget((usize, TypeId));

impl guinea_core::scope::Teardown for Forget {
    fn teardown(self) {
        let _ = MOUNTED.try_with(|mounted| mounted.borrow_mut().remove(&self.0));
    }
}

/// Runs `with` on the state kept for `scope`.
///
/// Taken out for the call and put back after it, rather than borrowed across
/// it: a segment draws its child inside its own drawing, and one that
/// navigates ends its own mount - after which there is nowhere to put
/// anything back, and the state goes with it.
///
/// A segment mounted with no `install` behind it - which a test does, and
/// nothing else - gets a default that lasts the call.
pub fn with<S: Default + 'static, R>(scope: &Scope, with: impl FnOnce(&mut S) -> R) -> R {
    let at = (scope.key(), TypeId::of::<S>());

    let taken = MOUNTED.with(|mounted| mounted.borrow_mut().get_mut(&at).and_then(Option::take));
    let mut state = taken
        .and_then(|state| state.downcast::<S>().ok())
        .map_or_else(S::default, |state| *state);

    let done = with(&mut state);

    MOUNTED.with(|mounted| {
        if let Some(slot) = mounted.borrow_mut().get_mut(&at) {
            *slot = Some(Box::new(state));
        }
    });

    done
}
