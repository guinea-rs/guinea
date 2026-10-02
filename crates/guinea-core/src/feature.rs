//! The two halves of a reducer's traffic, and the one call that wires them.
//!
//! What this replaces is four ways of saying the same thing. Ownership of a
//! reducer used to be a side effect of which of `port`, `actions`, `wire` or
//! `seed_reducer` a feature happened to call, and the two ends of one edge -
//! "this actor drives this reducer" and "this action goes to that actor" -
//! were declared in different places. A member nobody wired warned at runtime
//! instead of failing to build.
//!
//! Here the pair is known by construction. [`Claim::driven_by`] creates the
//! actor from the very expression that claims the reducer, and hands the
//! closure its [`Push`] as a parameter - so the direction is named where it is
//! used rather than in the name of a method that fetches it. Nothing is left
//! to wire, so nothing can be left unwired.

use std::rc::Rc;

use crate::actor::event_bus::EventBus;
use crate::actor::registry::DebugRegistry;
use crate::actor::{Addr, ManagedActor, UiThreadToken};
use crate::scope::{Reducer, Scope};

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
    fn new(scope: Scope) -> Self {
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
                reducer: crate::actor::short_type_name::<R>(),
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
#[derive(Clone, Default)]
pub struct Dispatch {
    /// One installed feature's corner of one scope. Named, not kept: a
    /// dispatcher a widget captured answers nothing once its page is removed.
    ///
    /// One and not a chain: which feature answers is settled by what the
    /// reader was reading, and reading already found the scope and the
    /// instance. Searching upward would make two instances of one feature
    /// indistinguishable again.
    at: Option<(Scope, usize)>,
}

impl Dispatch {
    /// The section that owns `R` - what a reader of `R` is handed.
    pub fn owning<R: 'static>(scope: Scope) -> Self {
        Self::in_section(scope, scope.section_of::<R>())
    }

    /// A named section - what a feature is handed while it is installing.
    pub(crate) fn in_section(scope: Scope, section: usize) -> Self {
        Self {
            at: Some((scope, section)),
        }
    }

    /// Hands the action to whatever answers it in that feature.
    pub fn emit<M: 'static>(&self, action: M) {
        let found = self
            .at
            .and_then(|(scope, section)| scope.answerer::<M>(section));

        match found {
            Some(answer) => {
                let _action = crate::trace::enter(|| crate::trace::Point::Action {
                    message: crate::actor::short_type_name::<M>(),
                });
                answer(action)
            }
            // The feature that owns what was read does not answer this - a
            // wiring mistake rather than something a user did, so it says what
            // went unanswered and lets the frame stand.
            None => tracing::warn!(
                action = crate::actor::short_type_name::<M>(),
                "the feature this dispatcher belongs to does not answer this action; dropped"
            ),
        }
    }
}

/// The reducers a feature lets other segments read.
///
/// `pub` for reducers: listed means a page below can read it, unlisted means
/// it is the feature's own business. A feature's internal state used to be
/// reachable from anywhere below simply because it existed, which is the
/// difference between a feature and a folder.
///
/// Implemented for `()` and for tuples of reducers, so a single export is
/// written `(Processes,)`. The trailing comma is the price of not having a
/// blanket impl over every reducer: a blanket one would overlap with the tuple
/// impls, since nothing stops a downstream crate implementing `Reducer` for a
/// tuple.
pub trait Exported {
    fn mark(scope: Scope);

    /// The first listed reducer this scope never claimed, if there is one.
    ///
    /// `Installs` closed the drift between what a segment says it installs and
    /// what it built, by making the list the body's return value. `Exports`
    /// cannot be a return value - it is read at build time, by
    /// [`Reaches`](../../guinea_app/feature/trait.Reaches.html), and a value
    /// arrives too late for that. So the same drift is closed from the other
    /// end: the list is checked against what was actually claimed, the moment
    /// the feature finishes installing.
    ///
    /// Without it, exporting something the feature never claimed type-checks,
    /// and a page below reads the reducer's `Default` forever - the state is
    /// created on first read, so nothing ever fails. Silence is the whole
    /// problem: a wrong export looks exactly like a feature that has not
    /// pushed an update yet.
    fn unclaimed(scope: Scope) -> Option<&'static str>;
}

fn missing<R: Reducer>(scope: Scope) -> Option<&'static str> {
    (!scope.claims::<R>()).then(|| std::any::type_name::<R>())
}

impl Exported for () {
    fn mark(_scope: Scope) {}

    fn unclaimed(_scope: Scope) -> Option<&'static str> {
        None
    }
}

macro_rules! exported {
    ($($reducer:ident),+) => {
        impl<$($reducer: Reducer),+> Exported for ($($reducer,)+) {
            fn mark(scope: Scope) {
                $(scope.note_export::<$reducer>();)+
            }

            fn unclaimed(scope: Scope) -> Option<&'static str> {
                None$(.or_else(|| missing::<$reducer>(scope)))+
            }
        }
    };
}

exported!(A);
exported!(A, B);
exported!(A, B, C);
exported!(A, B, C, D);
exported!(A, B, C, D, E);
exported!(A, B, C, D, E, F);
exported!(A, B, C, D, E, F, G);
exported!(A, B, C, D, E, F, G, H);
exported!(A, B, C, D, E, F, G, H, I);
exported!(A, B, C, D, E, F, G, H, I, J);
exported!(A, B, C, D, E, F, G, H, I, J, K);
exported!(A, B, C, D, E, F, G, H, I, J, K, L);

/// A domain that answers actions through an actor.
///
/// Written by `actor!` from the `handlers { .. }` it already lists, so the
/// registration cannot drift from the handlers and nothing can be left
/// unwired. Implementing it by hand is not the alternative to having an actor
/// - `cx.answers::<M>(..)` is.
pub trait Serves: Sized + 'static {
    fn serve(addr: &Addr<Self>, scope: Scope);
}

/// A reducer being claimed, and what may still be said about it.
///
/// Produced by `cx.state::<R>()` during install. Ending the chain without
/// [`driven_by`](Self::driven_by) is not a half-finished feature - it is state
/// the UI owns outright, and the type says so: nothing drives it, so nothing
/// can be emitted to it.
pub struct Claim<'a, R: Reducer> {
    scope: Scope,
    bus: &'a Rc<EventBus>,
    token: &'a UiThreadToken,
    debug: &'a Rc<DebugRegistry>,
    reducer: std::marker::PhantomData<fn() -> R>,
}

impl<'a, R: Reducer> Claim<'a, R> {
    /// For a context that hands features their scope - `FeatureInitContext`
    /// in `guinea-app`, and nothing else.
    pub fn new(
        scope: Scope,
        bus: &'a Rc<EventBus>,
        token: &'a UiThreadToken,
        debug: &'a Rc<DebugRegistry>,
    ) -> Self {
        scope.note_reducer_owner::<R>();
        Self {
            scope,
            bus,
            token,
            debug,
            reducer: std::marker::PhantomData,
        }
    }

    /// Starts from something other than `R::default()`.
    ///
    /// For a synchronous read - a setting already on disk - so the first frame
    /// shows real data instead of defaults followed by a round trip.
    pub fn seed(self, state: R) -> Self {
        if self.scope.peek::<R>().is_none() {
            self.scope.seed::<R>(state);
        }
        self
    }

    /// Creates the actor that drives this reducer, and installs it here.
    ///
    /// The closure is handed the [`Push`] rather than fetching it, and the
    /// actor's lifetime becomes the scope's - so a feature declares the whole
    /// edge in one expression and owns none of the bookkeeping. The actor type
    /// is inferred from what the closure returns; neither the reducer nor this
    /// call has to name it.
    ///
    /// The address comes back for wiring that is the actor's own, such as
    /// `addr.subscribe_on::<M>(Bus::Global)`. The scope still owns the actor
    /// and disposes it, and what it subscribed to ends with it.
    pub fn driven_by<A, F>(self, build: F) -> (Bound<R>, Addr<A>)
    where
        F: FnOnce(Push<R>) -> A,
        A: ManagedActor + Serves + std::fmt::Debug + 'static,
    {
        let actor = Addr::new_managed_scoped(build(Push::new(self.scope)), self.token.clone());
        actor.live_in(self.scope, self.bus);
        A::serve(&actor, self.scope);
        self.debug.register_owned(
            &actor,
            crate::actor::registry::Owner {
                scope: Some(self.scope.key()),
                feature: self.scope.current_feature(),
                drives: Some(crate::actor::short_type_name::<R>()),
            },
        );
        self.scope.own(Unregister {
            id: actor.id(),
            registry: self.debug.clone(),
        });
        self.scope.own(actor.clone());
        (self.bound(), actor)
    }

    /// Leaves it for the feature to drive however it likes.
    ///
    /// State the UI owns outright, or a domain that answers with something
    /// other than an actor - see `cx.answers::<M>(..)`.
    pub fn plain(self) -> Bound<R> {
        self.bound()
    }

    fn bound(self) -> Bound<R> {
        Bound {
            push: Push::new(self.scope),
            // The section being installed: a feature's own handle answers
            // through itself, not through whatever else the scope holds.
            dispatch: Dispatch::in_section(self.scope, self.scope.current_section()),
        }
    }
}

struct Unregister {
    id: usize,
    registry: Rc<DebugRegistry>,
}

impl crate::scope::Teardown for Unregister {
    fn teardown(self) {
        self.registry.unregister(self.id);
    }
}

/// What a feature keeps of a reducer it claimed.
///
/// Both directions, because a feature is the one place that legitimately has
/// both: it may change the state itself and it may ask its actor for
/// something. Cheap to hold - two handles, no state.
pub struct Bound<R: Reducer> {
    push: Push<R>,
    dispatch: Dispatch,
}

impl<R: Reducer> Clone for Bound<R> {
    fn clone(&self) -> Self {
        Self {
            push: self.push.clone(),
            dispatch: self.dispatch.clone(),
        }
    }
}

impl<R: Reducer> Bound<R> {
    pub fn push(&self, update: R::Update) {
        self.push.send(update);
    }

    pub fn emit<M: 'static>(&self, action: M) {
        self.dispatch.emit(action);
    }

    /// The way in, for handing to something that is not an actor - a timer, a
    /// stream, a callback from a library.
    pub fn port(&self) -> Push<R> {
        self.push.clone()
    }
}
