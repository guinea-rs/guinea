//! A list property that reads the reducer's state instead of copying it.
//!
//! The obvious binding - `model.set_items(state.items.to_slint())` - rebuilds
//! the whole list on every change: a `SharedString` per row, a fresh
//! `VecModel`, for a state that may have had one row added to it. And the rows
//! it builds are rows nobody is looking at, since a list shows a screenful.
//!
//! So instead the model *is* the state: [`Rows`] holds the same binding a
//! `bind` would, answers `row_count` from it, and converts a row only when
//! Slint asks for that row. What travels is one value at a time, on demand,
//! and `String` stays `String` everywhere outside this file - a Slint type in
//! the domain would be a worse trade than the copying it saves.

use std::rc::Rc;

use guinea_core::binding::ReducerBinding;
use guinea_core::scope::Reducer;
use slint::{Model, ModelNotify, ModelRc, ModelTracker};

use crate::ToSlint;

/// A [`Model`] over a slice of a reducer's state.
pub struct Rows<R: Reducer, T: ToSlint> {
    binding: ReducerBinding<R>,
    select: fn(&R) -> &[T],
    notify: ModelNotify,
}

impl<R, T> Rows<R, T>
where
    R: Reducer,
    T: ToSlint + 'static,
    T::Slint: Clone + 'static,
{
    /// Wraps the state behind `binding`, watching it for changes.
    ///
    /// The subscription lives as long as the scope, the way every other
    /// binding here does; the model itself lives as long as the property
    /// holding it.
    pub(crate) fn new(binding: ReducerBinding<R>, select: fn(&R) -> &[T]) -> ModelRc<T::Slint> {
        let rows = Rc::new(Self {
            binding: binding.clone(),
            select,
            notify: ModelNotify::default(),
        });

        binding.on_change_owned({
            let rows = rows.clone();
            // Which row changed is not something a reducer says - it replaces
            // the state - so the honest answer is that all of them did.
            move |_| rows.notify.reset()
        });

        ModelRc::from(rows as Rc<dyn Model<Data = T::Slint>>)
    }
}

impl<R, T> Model for Rows<R, T>
where
    R: Reducer,
    T: ToSlint + 'static,
    T::Slint: Clone + 'static,
{
    type Data = T::Slint;

    fn row_count(&self) -> usize {
        (self.select)(&self.binding.peek()).len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        (self.select)(&self.binding.peek())
            .get(row)
            .map(ToSlint::to_slint)
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

#[cfg(test)]
mod tests {
    use super::Rows;
    use guinea_core::scope::{Reducer, Scope, ScopeTree};
    use slint::Model;

    #[derive(Clone, Default, Debug)]
    struct Items(Vec<String>);

    impl Reducer for Items {
        type Update = Vec<String>;

        fn reduce(&mut self, items: Vec<String>) {
            self.0 = items;
        }
    }

    fn rows(scope: &Scope) -> slint::ModelRc<slint::SharedString> {
        Rows::new(scope.binding::<Items>(), |items: &Items| items.0.as_slice())
    }

    #[test]
    fn the_model_answers_from_the_state_it_was_given() {
        let scope = ScopeTree::new();
        scope.seed::<Items>(Items(vec!["systemd".to_string(), "sshd".to_string()]));

        let model = rows(&scope);

        assert_eq!(model.row_count(), 2);
        assert_eq!(model.row_data(1).unwrap(), "sshd");
    }

    #[test]
    fn a_change_to_the_state_is_a_change_to_the_model() {
        // Nothing was set on the model and nothing was copied into it: the
        // state moved, and the model reads the state.
        let scope = ScopeTree::new();
        scope.seed::<Items>(Items(vec!["systemd".to_string(), "sshd".to_string()]));

        let model = rows(&scope);
        scope.push::<Items>(vec!["systemd".to_string()]);

        assert_eq!(model.row_count(), 1);
        assert_eq!(model.row_data(0).unwrap(), "systemd");
        assert!(model.row_data(1).is_none());
    }
}
