use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::actor::shape::Declared;

use super::{Scope, ScopeData};

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

pub(super) struct Cell {
    state: Rc<dyn Any>,
    listeners: RefCell<Vec<(u64, Rc<dyn Fn()>)>>,
    observers: RefCell<Vec<(u64, Rc<dyn Fn(&dyn Any)>)>>,
}

/// What a cell holds, for printing it without knowing the type.
#[derive(Clone, Copy)]
pub(super) struct Kind {
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

impl Cell {
    fn empty<R: Reducer>() -> Self {
        Self::holding(Rc::new(Slot::new(Rc::new(R::default()))))
    }

    fn holding(state: Rc<dyn Any>) -> Self {
        Cell {
            state,
            listeners: RefCell::new(Vec::new()),
            observers: RefCell::new(Vec::new()),
        }
    }
}

impl Scope {
    /// Every reducer this scope holds whose type it has seen, printed.
    pub fn describe_states(&self) -> Vec<DescribedState> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.cells.borrow();
        let kinds = data.kinds.borrow();
        let owners = data.owners.borrow();
        let mut described: Vec<DescribedState> = cells
            .iter()
            .filter_map(|(type_id, cell)| {
                let kind = kinds.get(type_id)?;
                Some(DescribedState {
                    type_name: kind.name,
                    state: (kind.describe)(&*cell.state),
                    feature: owners
                        .get(type_id)
                        .and_then(|section| data.section_name(*section)),
                    declared: data.declarations.borrow().get(type_id).copied(),
                })
            })
            .collect();
        described.sort_by_key(|state| state.type_name);
        described
    }

    /// `R`'s state here, created from `R::default()` on first read.
    ///
    /// A removed scope has no state to hand out: what comes back is a fresh
    /// default nobody else sees.
    pub fn state<R: Reducer>(&self) -> Rc<Slot<R>> {
        match self.data() {
            Some(data) => data.state::<R>(),
            None => Rc::new(Slot::new(Rc::new(R::default()))),
        }
    }

    pub fn peek<R: Reducer>(&self) -> Option<Rc<Slot<R>>> {
        let data = self.data()?;
        data.note_kind::<R>();
        let cells = data.cells.borrow();
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
        let data = self.installing("seeding a reducer");
        data.note_kind::<R>();
        let replaced = data
            .cells
            .borrow_mut()
            .insert(TypeId::of::<R>(), Cell::holding(Rc::new(Slot::new(Rc::new(state)))));
        drop(replaced);
    }

    /// Applies `msg` to `F`'s state now, and marks the cell so its listeners
    /// run at the next [`notify::drain`](crate::notify::drain).
    ///
    /// The state is current the moment this returns; what waits is the
    /// redraw. Nothing a listener does - navigating, dropping scopes, sending
    /// to another actor - happens on the stack of whoever pushed. A no-op once
    /// the scope is removed.
    pub fn push<R: Reducer>(&self, update: R::Update) {
        let Some(data) = self.data() else { return };
        let carried: Option<Box<dyn Any>> = data
            .is_observed(TypeId::of::<R>())
            .then(|| Box::new(update.clone()) as Box<dyn Any>);
        let state = data.state::<R>();
        drop(data);

        {
            let mut state = state.borrow_mut();
            Rc::make_mut(&mut state).reduce(update);
        }

        crate::notify::mark(*self, TypeId::of::<R>(), carried);
    }

    /// Watches *what happened* to `F`, not merely that something did.
    ///
    /// A listener from [`subscribe`](Self::subscribe) learns the cell changed
    /// and reads the new state; an observer is handed the update itself, which
    /// is what tells a rename apart from a whole new list. Runs during the
    /// drain, before any listener, so state that depends on this one has
    /// settled by the time anything draws.
    pub fn observe<R: Reducer>(&self, callback: impl Fn(&R::Update) + 'static) -> Subscription {
        let Some(data) = self.data() else {
            return Subscription::inert();
        };
        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        {
            let mut cells = data.cells.borrow_mut();
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

        let scope = *self;
        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(data) = scope.data() else { return };
                if let Some(cell) = data.cells.borrow_mut().get_mut(&TypeId::of::<R>()) {
                    cell.observers.borrow_mut().retain(|(oid, _)| *oid != id);
                }
            })),
        }
    }

    pub(crate) fn listeners_of(&self, cell: TypeId) -> Vec<Rc<dyn Fn()>> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.listeners.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }

    pub(crate) fn observers_of(&self, cell: TypeId) -> Vec<Rc<dyn Fn(&dyn Any)>> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.observers.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }

    pub fn subscribe<R: Reducer>(&self, callback: impl Fn() + 'static) -> Subscription {
        let Some(data) = self.data() else {
            return Subscription::inert();
        };
        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        {
            let mut cells = data.cells.borrow_mut();
            let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
            cell.listeners.borrow_mut().push((id, Rc::new(callback)));
        }

        let scope = *self;
        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(data) = scope.data() else { return };
                if let Some(cell) = data.cells.borrow_mut().get_mut(&TypeId::of::<R>()) {
                    cell.listeners.borrow_mut().retain(|(lid, _)| *lid != id);
                }
            })),
        }
    }

    /// A handle to `R`'s state and actions in this scope, for code that reads
    /// or watches them without being a UI hook. See [`crate::binding`].
    pub fn binding<R: Reducer>(&self) -> crate::binding::ReducerBinding<R> {
        crate::binding::ReducerBinding::new(*self)
    }

    /// Returns a shallow clone of every reducer state currently held by this
    /// scope, keyed by the reducer's `TypeId`. Used by the router to cache
    /// page state in memory while the page is not mounted.
    pub fn snapshot_states(&self) -> HashMap<TypeId, Rc<dyn Any>> {
        let Some(data) = self.data() else {
            return HashMap::new();
        };
        data.cells
            .borrow()
            .iter()
            .map(|(type_id, cell)| (*type_id, cell.state.clone()))
            .collect()
    }

    /// Pre-populates reducer cells with previously cached states. This is the
    /// inverse of [`snapshot_states`](Self::snapshot_states): when a page is
    /// remounted, restoring its state before `install` runs lets the page find
    /// ready data instead of defaults.
    pub fn restore_states(&self, states: HashMap<TypeId, Rc<dyn Any>>) {
        let data = self.installing("restoring state");
        let mut replaced = Vec::new();
        {
            let mut cells = data.cells.borrow_mut();
            for (type_id, state) in states {
                replaced.extend(cells.insert(type_id, Cell::holding(state)));
            }
        }
        drop(replaced);
    }
}

impl ScopeData {
    fn note_kind<R: Reducer>(&self) {
        self.kinds
            .borrow_mut()
            .entry(TypeId::of::<R>())
            .or_insert_with(Kind::of::<R>);
    }

    fn state<R: Reducer>(&self) -> Rc<Slot<R>> {
        self.note_kind::<R>();
        let mut cells = self.cells.borrow_mut();
        let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
        cell.state
            .clone()
            .downcast::<Slot<R>>()
            .expect("Scope cell type mismatch for this TypeId - unreachable, keyed by R")
    }

    fn is_observed(&self, cell: TypeId) -> bool {
        self.cells
            .borrow()
            .get(&cell)
            .is_some_and(|cell| !cell.observers.borrow().is_empty())
    }
}
