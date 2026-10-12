use crate::actor::shape::Declared;
use crate::actor::{Addr, Home, ManagedActor};
use crate::scope::{Reducer, Section};

use super::{Dispatch, Push};

/// A domain that answers actions through an actor.
///
/// Written by `actor!` from the `handlers { .. }` it already lists, so the
/// registration cannot drift from the handlers and nothing can be left
/// unwired. Implementing it by hand is not the alternative to having an actor
/// - `cx.answers::<M>(..)` is.
pub trait Serves: Sized + 'static {
    fn serve(addr: &Addr<Self>, section: Section);
}

/// A reducer being claimed, and what may still be said about it.
///
/// Produced by `cx.state::<R>()` during install. Ending the chain without
/// [`driven_by`](Self::driven_by) is not a half-finished feature - it is state
/// the UI owns outright, and the type says so: nothing drives it, so nothing
/// can be emitted to it.
pub struct Claim<R: Reducer> {
    section: Section,
    reducer: std::marker::PhantomData<fn() -> R>,
}

impl<R: Reducer> Claim<R> {
    /// For a context that hands features their section - the contexts in
    /// `guinea-app`, and nothing else.
    pub fn new(section: Section, declared: Declared) -> Self {
        section.claim::<R>(Some(declared));
        Self {
            section,
            reducer: std::marker::PhantomData,
        }
    }

    /// Starts from something other than `R::default()`.
    ///
    /// For a synchronous read - a setting already on disk - so the first frame
    /// shows real data instead of defaults followed by a round trip.
    pub fn seed(self, state: R) -> Self {
        let scope = self.section.scope();
        if scope.peek::<R>().is_none() {
            scope.seed::<R>(state);
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
    /// What the actor hears on the bus is its manifest's `subscribes { .. }`,
    /// heard from the moment it is made. The scope owns the actor and
    /// disposes it, and what it hears ends with it.
    pub fn driven_by<A, F>(self, build: F) -> (Bound<R>, Addr<A>)
    where
        F: FnOnce(Push<R>) -> A,
        A: ManagedActor + Serves + 'static,
    {
        let home = Home::in_section(self.section);
        let drives = Some(crate::short_type_name::<R>());
        let actor = Addr::listed(build(Push::new(self.section.scope())), &home, drives);
        A::serve(&actor, self.section);
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
            push: Push::new(self.section.scope()),
            // The section that claimed it: a feature's own handle answers
            // through itself, not through whatever else the scope holds.
            dispatch: Dispatch::in_section(self.section),
        }
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
        *self
    }
}

impl<R: Reducer> Copy for Bound<R> {}

impl<R: Reducer> Bound<R> {
    pub fn push(&self, update: R::Update) {
        self.push.send(update);
    }

    pub fn emit<M: 'static>(&self, action: M) {
        self.dispatch.emit(action);
    }

    /// The way in, for handing to something that is not an actor - a timer, a
    /// stream, a callback from a library.
    pub fn pusher(&self) -> Push<R> {
        self.push
    }
}
