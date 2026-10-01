use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::task::JoinHandle;

use crate::actor::Addr;
use crate::actor::event_bus::subscribe::BusSubscription;
use crate::actor::shape::Declared;

static NEXT_SUBSCRIBER_ID: AtomicU64 = AtomicU64::new(0);

pub struct Subscription {
    unsubscribe: Option<Box<dyn FnOnce()>>,
}

impl Subscription {
    /// A subscription to something that no longer exists - dropping it does
    /// nothing.
    pub(crate) fn inert() -> Self {
        Self { unsubscribe: None }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(unsubscribe) = self.unsubscribe.take() {
            unsubscribe();
        }
    }
}

pub struct StateHandle<T>(Rc<RefCell<T>>);

impl<T> StateHandle<T> {
    pub fn borrow(&self) -> Ref<'_, T> {
        self.0.borrow()
    }
}

impl<T> Clone for StateHandle<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> From<Rc<RefCell<T>>> for StateHandle<T> {
    fn from(inner: Rc<RefCell<T>>) -> Self {
        Self(inner)
    }
}

/// State, and how it changes.
///
/// The type that implements this **is** the state - there is no `type State`,
/// because the struct is already there and already named. Two items, both
/// about state; a reducer cannot know who asked, only what changed.
///
/// ```ignore
/// #[derive(Clone, Debug, Default)]
/// pub struct Processes { pub items: Vec<String> }
///
/// pub enum Refreshed { Items(Vec<String>) }
///
/// impl Reducer for Processes {
///     type Update = Refreshed;
///
///     fn reduce(&mut self, update: Refreshed) {
///         match update { Refreshed::Items(items) => self.items = items }
///     }
/// }
/// ```
///
/// Usually it is written as a function instead - `#[reducer] fn processes(this:
/// &mut Processes, update: Refreshed)` - and the attribute writes this impl.
///
/// There is no actor here, and there must not be. A reducer knows its own
/// state and how it changes; naming the actor that happens to drive it would
/// put the domain's plumbing into the one declaration that is supposed to be
/// free of it. What relates an action to an actor is
/// [`Action`](crate::feature::Action), declared where actions already live.
///
/// `Clone` because a reader is handed the state as it is now, shared, and a
/// change made while a reader still holds it goes to a copy.
pub trait Reducer: Clone + Default + std::fmt::Debug + 'static {
    /// What changes it. `Clone` because an observer is handed the update
    /// itself, and `reduce` consumes it - the copy is made only when
    /// something is actually observing.
    type Update: Clone + 'static;

    fn reduce(&mut self, update: Self::Update);
}

/// Where a reducer's state lives: the current value, shared with whoever read
/// it last.
pub type Slot<R> = RefCell<Rc<R>>;

struct Cell {
    state: Rc<dyn Any>,
    listeners: RefCell<Vec<(u64, Rc<dyn Fn()>)>>,
    observers: RefCell<Vec<(u64, Rc<dyn Fn(&dyn Any)>)>>,
}

/// What a cell holds, for printing it without knowing the type.
#[derive(Clone, Copy)]
struct Kind {
    name: &'static str,
    describe: fn(&dyn Any) -> String,
}

impl Kind {
    fn of<R: Reducer>() -> Self {
        Self {
            name: std::any::type_name::<R>(),
            describe: |state| match state.downcast_ref::<Slot<R>>() {
                Some(cell) => match cell.try_borrow() {
                    Ok(state) => format!("{:#?}", **state),
                    Err(_) => "<being changed>".to_string(),
                },
                None => "<unknown>".to_string(),
            },
        }
    }
}

/// A subscription a feature made, as devtools see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Listener {
    pub event: &'static str,
    /// The actor that listens, or `None` for a feature's own callback.
    pub actor: Option<&'static str>,
    pub bus: crate::trace::Bus,
    pub feature: Option<&'static str>,
}

/// One reducer as devtools see it.
#[derive(Clone, Debug, PartialEq)]
pub struct DescribedState {
    pub type_name: &'static str,
    pub state: String,
    /// The feature that claimed it; `None` for the segment's own.
    pub feature: Option<&'static str>,
    /// Where it was claimed: the `cx.state::<R>()` that did it.
    pub declared: Option<Declared>,
}

/// A feature installed in a scope, as devtools see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Installed {
    pub name: &'static str,
    /// Where `impl Feature` was written, when `#[installs]` wrote it.
    pub declared: Option<Declared>,
}

impl Cell {
    fn empty<R: Reducer>() -> Self {
        Cell {
            state: Rc::new(Slot::new(Rc::new(R::default()))),
            listeners: RefCell::new(Vec::new()),
            observers: RefCell::new(Vec::new()),
        }
    }
}

#[derive(Default)]
pub struct Scope {
    cells: RefCell<HashMap<TypeId, Cell>>,
    /// Kept apart from `cells`: a cell restored from the state cache arrives
    /// erased, and learns its type again at the next typed access.
    kinds: RefCell<HashMap<TypeId, Kind>>,
    teardowns: RefCell<Vec<Box<dyn FnOnce()>>>,
    /// Features (identified by their `install` function's own type - see
    /// `FeatureInitContext::install`/`inherit` in the `guinea` crate) that
    /// were explicitly installed *in this scope*. Separate from `cells`
    /// (which tracks `Reducer` state/actions) because a feature can spawn
    /// several actors/reducers as one unit - this tracks the unit itself,
    /// so a descendant scope can find out "has an ancestor already taken
    /// ownership of this feature" without knowing which reducer types it
    /// happens to use internally.
    installed_features: RefCell<HashSet<TypeId>>,
    /// The reducers this scope lets segments below it read. See
    /// [`Scope::note_export`].
    ///
    /// Flat, unlike the answerers below: visibility is not per-instance, and
    /// two instances of one feature export two different reducer types anyway.
    exports: RefCell<HashSet<TypeId>>,
    /// What this scope answers, one map per installed feature.
    ///
    /// Not one map: an action type is the same for every instance of a
    /// feature, so `ListFeature<Recent>` and `ListFeature<Archived>` both
    /// answer `Refresh`, and a flat map would let whichever installed last
    /// answer for both. Section 0 is the segment's own, outside any feature.
    sections: RefCell<Vec<HashMap<TypeId, Rc<dyn Any>>>>,
    /// Which feature each section belongs to.
    section_names: RefCell<Vec<Option<&'static str>>>,
    /// Where each section's feature was written.
    section_declarations: RefCell<Vec<Option<Declared>>>,
    /// Where each reducer was claimed.
    declarations: RefCell<HashMap<TypeId, Declared>>,
    /// What this scope's features subscribed to.
    listeners: RefCell<Vec<Listener>>,
    /// Which section claimed each reducer - what turns "the state I was
    /// reading" into "the instance that owns it".
    owners: RefCell<HashMap<TypeId, usize>>,
    /// The sections currently being installed, innermost last. A feature is
    /// free to install another one.
    installing: RefCell<Vec<usize>>,
    /// Asked before this scope is torn down. See [`Scope::on_leave`].
    leave_guards: RefCell<Vec<Rc<dyn Fn() -> crate::guard::Verdict>>>,
    /// Set while a router keeps this scope without showing it. See
    /// [`Scope::sleep`].
    asleep: Rc<std::cell::Cell<bool>>,
    /// Run when it is shown again. See [`Scope::on_wake`].
    wake_hooks: RefCell<Vec<Rc<dyn Fn()>>>,
}

/// Whether a scope is awake, for something it owns to ask on its own.
///
/// Held rather than the scope itself: a timer or a subscription the scope owns
/// must not keep it alive.
#[derive(Clone)]
pub struct Awake(Rc<std::cell::Cell<bool>>);

impl Awake {
    /// Whether the scope is awake now.
    pub fn now(&self) -> bool {
        !self.0.get()
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        for teardown in self.teardowns.get_mut().drain(..) {
            teardown();
        }
    }
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks feature `F` (its `install` function, used purely as a type
    /// identity) as owned by this scope. `F` must not already be marked
    /// here - two different call sites both claiming ownership of the same
    /// feature in the same scope is a setup bug, not something to merge
    /// silently.
    pub fn mark_feature_installed<F: 'static>(&self) {
        let newly_inserted = self.installed_features.borrow_mut().insert(TypeId::of::<F>());
        assert!(
            newly_inserted,
            "feature already installed in this scope - install() called twice for the same feature"
        );
    }

    /// Marks reducer `R` as owned by this scope - same underlying set as
    /// [`mark_feature_installed`](Self::mark_feature_installed), keyed on
    /// `R` instead of an install-function type. Idempotent (unlike
    /// `mark_feature_installed`): a feature's actor constructor calling
    /// `ctx.port::<R>()` more than once (e.g. wiring two actors to the
    /// same reducer) is normal, not a setup bug - only the reducer-typed
    /// query this feeds (`resolve_owner`) cares whether it happened at
    /// all, not how many times.
    /// Whether anything in this scope has claimed `R`.
    ///
    /// What tells an export that was earned from one that was only declared.
    pub fn claims<R: 'static>(&self) -> bool {
        self.owners.borrow().contains_key(&TypeId::of::<R>())
    }

    pub fn note_reducer_owner<R: 'static>(&self) {
        self.installed_features.borrow_mut().insert(TypeId::of::<R>());
        self.owners
            .borrow_mut()
            .entry(TypeId::of::<R>())
            .or_insert_with(|| self.current_section());
    }

    /// Notes where reducer `R` was claimed, the first claim winning.
    pub fn note_reducer_declared<R: 'static>(&self, declared: Declared) {
        self.declarations.borrow_mut().entry(TypeId::of::<R>()).or_insert(declared);
    }

    /// The manifest directory of the feature installing now, for a claim that
    /// only knows the file the compiler gave it.
    pub fn current_crate_dir(&self) -> Option<&'static str> {
        let section = self.current_section();
        let declarations = self.section_declarations.borrow();
        declarations.get(section).copied().flatten().map(|declared| declared.crate_dir)
    }

    /// Opens a section for the feature `name` about to install. Returns its
    /// index.
    pub fn open_section(&self, name: &'static str, declared: Option<Declared>) -> usize {
        let mut sections = self.sections.borrow_mut();
        if sections.is_empty() {
            // Section 0: whatever the segment claims outside any feature.
            sections.push(HashMap::new());
        }
        sections.push(HashMap::new());
        let index = sections.len() - 1;
        let mut names = self.section_names.borrow_mut();
        names.resize(index + 1, None);
        names[index] = Some(name);

        let mut declarations = self.section_declarations.borrow_mut();
        declarations.resize(index + 1, None);
        declarations[index] = declared;

        self.installing.borrow_mut().push(index);
        index
    }

    /// The feature a section belongs to; `None` for the segment's own.
    pub fn section_name(&self, section: usize) -> Option<&'static str> {
        self.section_names.borrow().get(section).copied().flatten()
    }

    /// Every feature installed here, in the order they were.
    pub fn features(&self) -> Vec<Installed> {
        let declarations = self.section_declarations.borrow();

        self.section_names
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(section, name)| {
                Some(Installed {
                    name: (*name)?,
                    declared: declarations.get(section).copied().flatten(),
                })
            })
            .collect()
    }

    /// The feature being installed right now, if any.
    pub fn current_feature(&self) -> Option<&'static str> {
        self.section_name(self.current_section())
    }

    /// Notes that whatever is installing listens to `event` on `bus` - through
    /// `actor` when an actor does the listening.
    pub fn note_listener(
        &self,
        event: &'static str,
        actor: Option<&'static str>,
        bus: crate::trace::Bus,
    ) {
        self.listeners.borrow_mut().push(Listener {
            event,
            actor,
            bus,
            feature: self.current_feature(),
        });
    }

    pub fn listeners(&self) -> Vec<Listener> {
        self.listeners.borrow().clone()
    }

    /// Identifies this scope for as long as it lives.
    pub fn key(self: &Rc<Self>) -> usize {
        Rc::as_ptr(self) as usize
    }

    pub fn close_section(&self) {
        self.installing.borrow_mut().pop();
    }

    /// The section being installed, or the segment's own when none is.
    pub fn current_section(&self) -> usize {
        self.installing.borrow().last().copied().unwrap_or(0)
    }

    /// Which section owns `R` - the instance whose dispatcher a reader of `R`
    /// should be handed.
    pub fn section_of<R: 'static>(&self) -> usize {
        self.owners
            .borrow()
            .get(&TypeId::of::<R>())
            .copied()
            .unwrap_or(0)
    }

    /// Whether feature `F` was marked installed in *this exact* scope.
    pub fn has_feature<F: 'static>(&self) -> bool {
        self.installed_features.borrow().contains(&TypeId::of::<F>())
    }

    /// Marks `R` as readable from segments below this one.
    ///
    /// Called by `cx.install::<F>()` for everything in `F::Exports`, and by
    /// nothing else. A reducer a feature claimed but did not export stays
    /// visible to the feature itself and invisible from below - which is the
    /// whole difference between a feature and a folder.
    pub fn note_export<R: 'static>(&self) {
        self.exports.borrow_mut().insert(TypeId::of::<R>());
    }

    /// Whether `R` is readable from below this scope.
    pub fn exports<R: 'static>(&self) -> bool {
        self.exports.borrow().contains(&TypeId::of::<R>())
    }

    fn note_kind<R: Reducer>(&self) {
        self.kinds
            .borrow_mut()
            .entry(TypeId::of::<R>())
            .or_insert_with(Kind::of::<R>);
    }

    /// Every reducer this scope holds whose type it has seen, printed.
    pub fn describe_states(&self) -> Vec<DescribedState> {
        let cells = self.cells.borrow();
        let kinds = self.kinds.borrow();
        let owners = self.owners.borrow();
        let mut described: Vec<DescribedState> = cells
            .iter()
            .filter_map(|(type_id, cell)| {
                let kind = kinds.get(type_id)?;
                Some(DescribedState {
                    type_name: kind.name,
                    state: (kind.describe)(&*cell.state),
                    feature: owners
                        .get(type_id)
                        .and_then(|section| self.section_name(*section)),
                    declared: self.declarations.borrow().get(type_id).copied(),
                })
            })
            .collect();
        described.sort_by_key(|state| state.type_name);
        described
    }

    pub fn state<R: Reducer>(&self) -> Rc<Slot<R>> {
        self.note_kind::<R>();
        let mut cells = self.cells.borrow_mut();
        let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
        cell.state
            .clone()
            .downcast::<Slot<R>>()
            .expect("Scope cell type mismatch for this TypeId - unreachable, keyed by R")
    }

    pub fn peek<R: Reducer>(&self) -> Option<Rc<Slot<R>>> {
        self.note_kind::<R>();
        let cells = self.cells.borrow();
        let cell = cells.get(&TypeId::of::<R>())?;
        Some(
            cell.state
                .clone()
                .downcast::<Slot<R>>()
                .expect("Scope cell type mismatch for this TypeId - unreachable, keyed by R"),
        )
    }

    /// Sets `R`'s starting value instead of `R::default()`. Call before
    /// anything else touches `R` in this scope - overwrites any existing cell,
    /// dropping its listeners.
    pub fn seed<R: Reducer>(&self, state: R) {
        self.note_kind::<R>();
        self.cells.borrow_mut().insert(
            TypeId::of::<R>(),
            Cell {
                state: Rc::new(Slot::new(Rc::new(state))),
                listeners: RefCell::new(Vec::new()),
                observers: RefCell::new(Vec::new()),
            },
        );
    }

    /// Says that this scope answers `M`, and how.
    ///
    /// Keyed by the action, not by whoever answers it - which is what keeps
    /// the answerer out of every signature the UI touches. `actor!` calls this
    /// for each handler it lists; a domain that runs on tasks, or a channel,
    /// or a plain closure over a `RefCell`, calls it itself.
    pub fn answers<M: 'static>(&self, answer: impl Fn(M) + 'static) {
        let answer: Rc<dyn Fn(M)> = Rc::new(answer);
        let section = self.current_section();

        let mut sections = self.sections.borrow_mut();
        while sections.len() <= section {
            sections.push(HashMap::new());
        }
        sections[section].insert(TypeId::of::<M>(), Rc::new(answer) as Rc<dyn Any>);
    }

    /// What answers `M` in one section of this scope, if anything does.
    pub fn answerer<M: 'static>(
        &self,
        section: usize,
    ) -> Option<Rc<dyn Fn(M)>> {
        let sections = self.sections.borrow();
        let answer = sections.get(section)?.get(&TypeId::of::<M>())?.clone();
        answer.downcast::<Rc<dyn Fn(M)>>().ok().map(|a| (*a).clone())
    }

    /// What answers `M` anywhere in this scope - the first feature that does,
    /// in the order they installed. For a sender that knows the action and
    /// not which state it was reading.
    pub fn first_answerer<M: 'static>(&self) -> Option<Rc<dyn Fn(M)>> {
        let sections = self.sections.borrow().len();
        (0..sections).find_map(|section| self.answerer::<M>(section))
    }

    /// Applies `msg` to `F`'s state now, and marks the cell so its listeners
    /// run at the next [`notify::drain`](crate::notify::drain).
    ///
    /// The state is current the moment this returns; what waits is the
    /// redraw. Nothing a listener does - navigating, dropping scopes, sending
    /// to another actor - happens on the stack of whoever pushed.
    pub fn push<R: Reducer>(self: &Rc<Self>, update: R::Update) {
        let carried: Option<Box<dyn Any>> = self
            .is_observed(TypeId::of::<R>())
            .then(|| Box::new(update.clone()) as Box<dyn Any>);

        let state = self.state::<R>();
        {
            let mut state = state.borrow_mut();
            Rc::make_mut(&mut state).reduce(update);
        }

        crate::notify::mark(self, TypeId::of::<R>(), carried);
    }

    /// Watches *what happened* to `F`, not merely that something did.
    ///
    /// A listener from [`subscribe`](Self::subscribe) learns the cell changed
    /// and reads the new state; an observer is handed the update itself, which
    /// is what tells a rename apart from a whole new list. Runs during the
    /// drain, before any listener, so state that depends on this one has
    /// settled by the time anything draws.
    pub fn observe<R: Reducer>(
        self: &Rc<Self>,
        callback: impl Fn(&R::Update) + 'static,
    ) -> Subscription {
        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        {
            let mut cells = self.cells.borrow_mut();
            let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
            cell.observers.borrow_mut().push((
                id,
                Rc::new(move |update: &dyn Any| {
                    if let Some(update) = update.downcast_ref::<R::Update>() {
                        callback(update);
                    }
                }),
            ));
        }

        let scope = Rc::downgrade(self);
        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(scope) = scope.upgrade() else { return };
                if let Some(cell) = scope.cells.borrow_mut().get_mut(&TypeId::of::<R>()) {
                    cell.observers.borrow_mut().retain(|(oid, _)| *oid != id);
                }
            })),
        }
    }

    fn is_observed(&self, cell: TypeId) -> bool {
        self.cells
            .borrow()
            .get(&cell)
            .is_some_and(|cell| !cell.observers.borrow().is_empty())
    }

    pub(crate) fn listeners_of(&self, cell: TypeId) -> Vec<Rc<dyn Fn()>> {
        let cells = self.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.listeners.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }

    pub(crate) fn observers_of(&self, cell: TypeId) -> Vec<Rc<dyn Fn(&dyn Any)>> {
        let cells = self.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.observers.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }
    pub fn subscribe<R: Reducer>(
        self: &Rc<Self>,
        callback: impl Fn() + 'static,
    ) -> Subscription {
        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        {
            let mut cells = self.cells.borrow_mut();
            let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
            cell.listeners.borrow_mut().push((id, Rc::new(callback)));
        }

        let scope = Rc::downgrade(self);
        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(scope) = scope.upgrade() else { return };
                if let Some(cell) = scope.cells.borrow_mut().get_mut(&TypeId::of::<R>()) {
                    cell.listeners.borrow_mut().retain(|(lid, _)| *lid != id);
                }
            })),
        }
    }

    /// A handle to `R`'s state and actions in this scope, for code that reads
    /// or watches them without being a UI hook. See [`crate::binding`].
    pub fn binding<R: Reducer>(self: &Rc<Self>) -> crate::binding::ReducerBinding<R> {
        crate::binding::ReducerBinding::new(self)
    }

    pub fn own_subscription(&self, subscription: BusSubscription) {
        self.teardowns
            .borrow_mut()
            .push(Box::new(move || drop(subscription)));
    }

    /// Returns a shallow clone of every reducer state currently held by this
    /// scope, keyed by the reducer's `TypeId`. Used by the router to cache
    /// page state in memory while the page is not mounted.
    pub fn snapshot_states(&self) -> HashMap<TypeId, Rc<dyn Any>> {
        self.cells
            .borrow()
            .iter()
            .map(|(type_id, cell)| (*type_id, cell.state.clone()))
            .collect()
    }

    /// Pre-populates reducer cells with previously cached states. This is the
    /// inverse of [`snapshot_states`]: when a page is remounted, restoring its
    /// state before `install` runs lets the page find ready data instead of
    /// defaults.
    pub fn restore_states(&self, states: HashMap<TypeId, Rc<dyn Any>>) {
        let mut cells = self.cells.borrow_mut();
        for (type_id, state) in states {
            cells.insert(
                type_id,
                Cell {
                    state,
                    listeners: RefCell::new(Vec::new()),
                    observers: RefCell::new(Vec::new()),
                },
            );
        }
    }

    /// Binds any [`Teardown`] resource to this scope's lifetime: it is torn
    /// down when the scope is.
    pub fn own<R: Teardown>(&self, resource: R) {
        self.teardowns.borrow_mut().push(Box::new(move || resource.teardown()));
    }

    /// Asked before this scope is torn down by a navigation.
    ///
    /// Registered during `install`, which is the whole reason leaving and
    /// entering are declared in different places: on the way out the scope
    /// exists, so the guard can read its own state - which is what "unsaved
    /// changes" is. On the way in there is nothing to read yet.
    pub fn on_leave(&self, guard: impl Fn() -> crate::guard::Verdict + 'static) {
        self.leave_guards.borrow_mut().push(Rc::new(guard));
    }

    /// The guards to ask, in the order they were registered.
    ///
    /// Cloned out rather than borrowed: a guard is free to touch this scope,
    /// and the caller runs them while deciding.
    pub fn leave_guards(&self) -> Vec<Rc<dyn Fn() -> crate::guard::Verdict>> {
        self.leave_guards.borrow().clone()
    }

    /// Puts this scope to sleep: kept, with its state and actors, but deaf.
    ///
    /// What it owns stops hearing anything while it sleeps - its timers skip
    /// their ticks, its actors and callbacks are not told what the buses
    /// carry - and nothing is queued for later: what happened meanwhile is
    /// missed. A router does this to a `keep` segment it leaves.
    pub fn sleep(&self) {
        self.asleep.set(true);
    }

    /// Wakes this scope, then runs what asked to hear of it, in the order it
    /// asked.
    pub fn wake(&self) {
        self.asleep.set(false);

        let hooks = self.wake_hooks.borrow().clone();
        for hook in hooks {
            hook();
        }
    }

    pub fn is_awake(&self) -> bool {
        !self.asleep.get()
    }

    /// Whether this scope is awake, for what it owns to ask later.
    pub fn awake(&self) -> Awake {
        Awake(self.asleep.clone())
    }

    /// Runs `hook` every time this scope wakes: for a feature to catch up on
    /// what it missed while it slept.
    pub fn on_wake(&self, hook: impl Fn() + 'static) {
        self.wake_hooks.borrow_mut().push(Rc::new(hook));
    }
}

/// A resource whose lifetime can be bound to a [`Scope`] via [`Scope::own`].
/// Implement per resource kind; the "blanket" over arbitrary `T` is the
/// [`DropGuard`] newtype, so specialized teardowns never overlap.
pub trait Teardown: 'static {
    fn teardown(self);
}

impl Teardown for JoinHandle<()> {
    fn teardown(self) {
        self.abort();
    }
}

impl<A: 'static> Teardown for Addr<A> {
    fn teardown(self) {
        self.dispose();
        drop(self);
    }
}


/// Blanket teardown for resources that just need dropping
/// (`scope.own(DropGuard(resource))` when no specialized impl exists).
pub struct DropGuard<T: 'static>(pub T);

impl<T: 'static> Teardown for DropGuard<T> {
    fn teardown(self) {
        drop(self.0);
    }
}

pub struct GlobalScope;

impl GlobalScope {
    pub fn instance() -> Rc<Scope> {
        thread_local! {
            static SCOPE: Rc<Scope> = Rc::new(Scope::new());
        }
        SCOPE.with(|scope| scope.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default, Clone, PartialEq, Debug)]
    struct Counter {
        value: i32,
    }

    #[derive(Clone)]
    enum CounterMsg {
        Set(i32),
    }

    impl Reducer for Counter {
        type Update = CounterMsg;

        fn reduce(&mut self, update: CounterMsg) {
            match update {
                CounterMsg::Set(v) => self.value = v,
            }
        }
    }

    #[test]
    fn a_described_state_names_the_feature_that_claimed_it() {
        #[derive(Clone, Default, Debug)]
        struct Loose;

        impl Reducer for Loose {
            type Update = ();

            fn reduce(&mut self, _: ()) {}
        }

        let scope = Scope::new();
        scope.open_section("app::CounterFeature", None);
        scope.note_reducer_owner::<Counter>();
        scope.state::<Counter>();
        scope.close_section();
        scope.state::<Loose>();

        let described = scope.describe_states();
        let feature = |name: &str| {
            described
                .iter()
                .find(|state| state.type_name.ends_with(name))
                .map(|state| state.feature)
        };
        assert_eq!(feature("Counter"), Some(Some("app::CounterFeature")));
        assert_eq!(feature("Loose"), Some(None));
    }

    #[test]
    fn state_survives_unmount_remount_within_a_live_store() {
        let store = Rc::new(Scope::new());

        // "mount" #1: read default, then a push arrives.
        let first_read = store.state::<Counter>();
        assert_eq!(first_read.borrow().value, 0);
        store.push::<Counter>(CounterMsg::Set(42));

        // "unmount": drop the only handle a view held.
        drop(first_read);

        // "remount": a fresh `use_feature` call resolves the cell again -
        // must see the value left behind, not a new default.
        let second_read = store.state::<Counter>();
        assert_eq!(second_read.borrow().value, 42);
    }

    #[test]
    fn push_notifies_subscribers_and_unsubscribe_stops_it() {
        let store = Rc::new(Scope::new());
        let seen = Rc::new(RefCell::new(Vec::new()));

        let seen_for_sub = seen.clone();
        let store_for_sub = store.clone();
        let sub = store.subscribe::<Counter>(move || {
            seen_for_sub.borrow_mut().push(store_for_sub.state::<Counter>().borrow().value);
        });

        store.push::<Counter>(CounterMsg::Set(1));
        store.push::<Counter>(CounterMsg::Set(2));
        assert_eq!(*seen.borrow(), vec![1, 2]);

        drop(sub);
        store.push::<Counter>(CounterMsg::Set(3));
        assert_eq!(
            *seen.borrow(),
            vec![1, 2],
            "no further notifications after the Subscription is dropped"
        );
    }

    #[tokio::test]
    async fn dropping_the_store_aborts_owned_tasks() {
        let ran_to_completion = Arc::new(AtomicBool::new(false));
        let flag = ran_to_completion.clone();

        let store = Scope::new();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            flag.store(true, Ordering::SeqCst);
        });
        store.own(handle);

        // Scope teardown, well before the task's sleep elapses.
        drop(store);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !ran_to_completion.load(Ordering::SeqCst),
            "task should have been aborted when its owning cell was dropped"
        );
    }

    #[test]
    fn own_actor_disposes_the_registry_entry_on_cell_drop() {
        let token = crate::actor::UiThreadToken::dangerously_create_token_unchecked();
        let addr = Addr::new_scoped((), token);
        let counter = addr.strong_count_ptr();

        let store = Scope::new();
        store.own(addr.clone());
        drop(addr);

        // Only our own local `counter` handle plus the REGISTRY's - the
        // Scope hasn't torn down yet, so it hasn't disposed it.
        assert!(
            Rc::strong_count(&counter) > 1,
            "REGISTRY should still hold the actor alive while its Scope is alive"
        );

        drop(store);

        assert_eq!(
            Rc::strong_count(&counter),
            1,
            "dropping the Scope's cell should dispose the REGISTRY entry, \
             leaving only this test's own counter handle"
        );
    }
}
