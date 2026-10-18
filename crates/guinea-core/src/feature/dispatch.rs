use crate::scope::{Reducer, Scope, Section};

use super::UiThread;

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

impl crate::inspect::Inspect for Dispatch {
    fn inspect(&self, at: crate::inspect::Inspector<'_>) {
        at.unit("Dispatch");
    }
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
                let _recording = scope.tree_sink().map(|sink| crate::trace::record_into(&sink));
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
