use crate::scope::{Reducer, Scope, Section};

/// The way back into a reducer, for whoever changes it.
///
/// Handed to an actor as a parameter rather than fetched by name. It names its
/// scope and does not keep it: the scope goes when it is removed, and this
/// quietly stops.
pub struct Push<R: Reducer> {
    scope: Scope,
    reducer: std::marker::PhantomData<R>,
}

impl<R: Reducer> Clone for Push<R> {
    fn clone(&self) -> Self {
        Self {
            scope: self.scope,
            reducer: std::marker::PhantomData,
        }
    }
}

impl<R: Reducer> std::fmt::Debug for Push<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push")
            .field("reducer", &std::any::type_name::<R>())
            .field("scope_alive", &self.scope.is_alive())
            .finish()
    }
}

impl<R: Reducer> Push<R> {
    /// The way into `R` in `scope` - for a test that drives an actor without
    /// installing the feature around it.
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            reducer: std::marker::PhantomData,
        }
    }

    /// Applies an update. A no-op once the scope is gone, which is what
    /// happens to work an actor finishes after its page has been left.
    pub fn send(&self, update: R::Update) {
        if self.scope.is_alive() {
            crate::trace::mark(|| crate::trace::Point::Push {
                reducer: crate::short_type_name::<R>(),
            });
            self.scope.push::<R>(update);
        }
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
#[derive(Clone)]
pub struct Dispatch {
    /// One installed feature's corner of one scope. Named, not kept: a
    /// dispatcher a widget captured answers nothing once its page is removed.
    ///
    /// One and not a chain: which feature answers is settled by what the
    /// reader was reading, and reading already found the scope and the
    /// instance. Searching upward would make two instances of one feature
    /// indistinguishable again.
    at: Section,
}

impl Dispatch {
    /// The section that owns `R` - what a reader of `R` is handed.
    pub fn owning<R: Reducer>(scope: Scope) -> Self {
        Self::in_section(scope.section_of::<R>())
    }

    /// A named section - what a feature is handed for what it claimed.
    pub(crate) fn in_section(section: Section) -> Self {
        Self { at: section }
    }

    /// Hands the action to whatever answers it in that feature. Dropped once
    /// its scope is gone: a click on the last frame of a page that has left.
    pub fn emit<M: 'static>(&self, action: M) {
        if !self.at.scope().is_alive() {
            return;
        }

        match self.at.answerer::<M>() {
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
