use crate::scope::{Reducer, Scope, Section};

/// The way back into a reducer, for whoever changes it.
///
/// Handed to an actor as a parameter rather than fetched by name. It names its
/// scope and does not keep it: the scope goes when it is removed, and with it
/// the actor and whatever else was meant to push.
pub struct Push<R: Reducer> {
    scope: Scope,
    reducer: std::marker::PhantomData<R>,
    ui_thread: UiThread,
}

/// Keeps a handle on the thread its scope lives on: neither `Send` nor
/// `Sync`.
type UiThread = std::marker::PhantomData<*const ()>;

impl<R: Reducer> Clone for Push<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Reducer> Copy for Push<R> {}

impl<R: Reducer> std::fmt::Debug for Push<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push")
            .field("reducer", &std::any::type_name::<R>())
            .field("scope_alive", &self.scope.is_alive())
            .finish()
    }
}

impl<R: Reducer> crate::inspect::Inspect for Push<R> {
    fn inspect(&self, at: crate::inspect::Inspector<'_>) {
        at.unit(&format!("Push<{}>", crate::short_type_name::<R>()));
    }
}

impl crate::inspect::Inspect for Dispatch {
    fn inspect(&self, at: crate::inspect::Inspector<'_>) {
        at.unit("Dispatch");
    }
}

impl<R: Reducer> Push<R> {
    /// The way into `R` in `scope` - for a test that drives an actor without
    /// installing the feature around it.
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            reducer: std::marker::PhantomData,
            ui_thread: std::marker::PhantomData,
        }
    }

    /// Applies an update.
    ///
    /// # Panics
    ///
    /// When its scope is gone: whatever still pushes outlived the page it
    /// was pushing to.
    pub fn send(&self, update: R::Update) {
        crate::trace::mark(|| crate::trace::Point::Push {
            reducer: crate::short_type_name::<R>(),
        });
        self.scope.push::<R>(update);
    }
}

/// What the UI may ask of the features a segment can reach.
///
/// `emit` takes the action by value, so what is happening is readable where it
/// happens. Nothing comes back: the answer arrives as an update to a reducer.
/// That is why everything the UI needs has to be state - "compute this for me
/// on demand" does not exist - and the same fact is why teardown is safe,
/// since nobody is ever waiting on something that has gone.
///
/// **No actor appears anywhere in this path.** An action is a value the domain
/// answers; *how* it answers - an actor, a task with a `RefCell`, a channel to
/// a thread - is the domain's own business and may change without anything
/// outside noticing. An actor named in a signature would be exactly the leak
/// this avoids, and the compiler says so out loud: it makes the actor public.
///
/// What is still checked at build time is the part that matters: `actor!`
/// asserts `Handler<M>` for every action it lists, so an action nobody answers
/// fails to compile *inside the domain*. What is left to run time is whether
/// the feature is installed here at all - which was never a type's question.
#[derive(Clone, Copy)]
pub struct Dispatch {
    /// One installed feature's corner of one scope. Named, not kept: a
    /// dispatcher a widget captured answers nothing once its page is removed.
    ///
    /// One and not a chain: which feature answers is settled by what the
    /// reader was reading, and reading already found the scope and the
    /// instance. Searching upward would make two instances of one feature
    /// indistinguishable again.
    ///
    /// `None` only for [`Dispatch::nowhere`].
    at: Option<Section>,
    ui_thread: UiThread,
}

impl Dispatch {
    /// The section that owns `R` - what a reader of `R` is handed.
    pub fn owning<R: Reducer>(scope: Scope) -> Self {
        Self::in_section(scope.section_of::<R>())
    }

    /// A dispatcher that asks nobody, for a view drawn without a domain - a
    /// benchmark, a preview. What is emitted to it is dropped.
    pub fn nowhere() -> Self {
        Self {
            at: None,
            ui_thread: std::marker::PhantomData,
        }
    }

    /// A named section - what a feature is handed for what it claimed.
    pub(crate) fn in_section(section: Section) -> Self {
        Self {
            at: Some(section),
            ui_thread: std::marker::PhantomData,
        }
    }

    /// Hands the action to whatever answers it in that feature.
    ///
    /// # Panics
    ///
    /// When its scope is gone: whoever still emits to a page that has left
    /// is holding on to it past its end.
    pub fn emit<M: 'static>(&self, action: M) {
        let Some(section) = self.at else { return };

        let scope = section.scope();
        if !scope.is_alive() {
            panic!(
                "emitting {} in {scope:?}, which was removed",
                crate::short_type_name::<M>()
            );
        }

        match section.answerer::<M>() {
            Some(answer) => {
                let _action = crate::trace::enter(|| crate::trace::Point::Action {
                    message: crate::short_type_name::<M>(),
                });
                answer(action)
            }
            // The feature that owns what was read does not answer this - a
            // wiring mistake rather than something a user did, so it says what
            // went unanswered and lets the frame stand.
            None => tracing::warn!(
                action = crate::short_type_name::<M>(),
                "the feature this dispatcher belongs to does not answer this action; dropped"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::marker::PhantomData;

    struct Probe<T>(PhantomData<T>);

    #[allow(dead_code)]
    trait NotCopy {
        fn is_copy(&self) -> bool {
            false
        }
    }

    impl<T> NotCopy for Probe<T> {}

    impl<T: Copy> Probe<T> {
        fn is_copy(&self) -> bool {
            true
        }
    }

    #[allow(dead_code)]
    trait Stays {
        fn is_send(&self) -> bool {
            false
        }
    }

    impl<T> Stays for Probe<T> {}

    #[allow(dead_code)]
    impl<T: Send> Probe<T> {
        fn is_send(&self) -> bool {
            true
        }
    }

    #[derive(Clone, Default, Debug)]
    struct Count(u32);

    #[guinea_macros::reducer]
    fn count(this: &mut Count, by: u32) {
        this.0 += by;
    }

    #[test]
    fn a_push_is_handed_on_by_copy_whatever_it_pushes_into() {
        assert!(Probe::<Push<Count>>(PhantomData).is_copy());
    }

    #[test]
    fn a_dispatcher_is_handed_on_by_copy() {
        assert!(Probe::<Dispatch>(PhantomData).is_copy());
    }

    #[test]
    fn a_push_and_a_dispatcher_stay_on_the_thread_their_scope_lives_on() {
        assert_eq!(
            (
                Probe::<Push<Count>>(PhantomData).is_send(),
                Probe::<Dispatch>(PhantomData).is_send()
            ),
            (false, false)
        );
    }

    #[test]
    fn what_a_feature_keeps_of_its_reducer_is_handed_on_by_copy() {
        assert!(Probe::<crate::feature::Bound<Count>>(PhantomData).is_copy());
    }
}
