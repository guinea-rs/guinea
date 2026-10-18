use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::actor::shape::Declared;
use crate::inspect::{Inspector, Node, Open};
use crate::observability::observer;

use super::Scope;

static NEXT_SUBSCRIBER_ID: AtomicU64 = AtomicU64::new(0);

pub struct Subscription {
    unsubscribe: Option<Box<dyn FnOnce()>>,
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
/// because the struct is already there and already named. A reducer cannot
/// know who asked, only what changed.
///
/// It is written as a function, and `#[reducer]` writes the impl:
///
/// ```
/// # use guinea_core::scope::Reducer;
/// use guinea_macros::{Inspect, reducer};
///
/// #[derive(Clone, Default, Inspect)]
/// pub struct Processes { pub items: Vec<String> }
///
/// #[derive(Clone)]
/// pub enum Refreshed { Items(Vec<String>) }
///
/// #[reducer]
/// fn processes(this: &mut Processes, update: Refreshed) {
///     match update { Refreshed::Items(items) => this.items = items }
/// }
/// # let mut processes = Processes::default();
/// # processes.reduce(Refreshed::Items(vec!["guinea".into()]));
/// # assert_eq!(processes.items, ["guinea"]);
/// ```
///
/// Devtools show the state through its [`Inspect`](crate::inspect::Inspect)
/// when it has one, through its `Debug` when not, and by its name without
/// either: `#[reducer]` picks which, where the type is known.
///
/// There is no actor here, and there must not be. A reducer knows its own
/// state and how it changes; naming the actor that happens to drive it would
/// put the domain's plumbing into the one declaration that is supposed to be
/// free of it. What relates an action to an actor is
/// [`Action`](crate::feature::Action), declared where actions already live.
///
/// `Clone` because a reader is handed the state as it is now, shared, and a
/// change made while a reader still holds it goes to a copy.
pub trait Reducer: Clone + Default + 'static {
    /// What changes it. `Clone` because an observer is handed the update
    /// itself, and `reduce` consumes it - the copy is made only when
    /// something is actually observing.
    type Update: Clone + 'static;

    fn reduce(&mut self, update: Self::Update);

    /// How devtools show it; `#[reducer]` writes it.
    #[doc(hidden)]
    fn show(&self, at: Inspector<'_>);
}

/// Where a reducer's state lives: the current value, shared with whoever read
/// it last.
pub type Slot<R> = RefCell<Rc<R>>;

/// The reducers a scope holds.
#[derive(Default)]
pub(super) struct States {
    cells: RefCell<HashMap<TypeId, Cell>>,
    /// Kept apart from `cells`: a cell restored from the state cache arrives
    /// erased, and learns its type again at the next typed access.
    kinds: RefCell<HashMap<TypeId, Kind>>,
}

struct Cell {
    state: Rc<dyn Any>,
    readers: RefCell<Vec<(u64, Rc<Watcher<dyn Any>>)>>,
    observers: RefCell<Vec<(u64, Rc<Watcher<dyn Fn(&dyn Any)>>)>>,
}

/// A reader or an observer, and whether it still is one: a drain runs a
/// copy of the list, and what was dropped since must not run from it.
pub(crate) struct Watcher<F: ?Sized> {
    gone: std::cell::Cell<bool>,
    pub(crate) run: F,
}

impl<F> Watcher<F> {
    fn new(run: F) -> Self {
        Self {
            gone: std::cell::Cell::new(false),
            run,
        }
    }
}

impl<F: ?Sized> Watcher<F> {
    pub(crate) fn is_gone(&self) -> bool {
        self.gone.get()
    }

    fn leave(&self) {
        self.gone.set(true);
    }
}

impl Drop for Cell {
    fn drop(&mut self) {
        for (_, reader) in self.readers.get_mut().iter() {
            reader.leave();
        }

        for (_, observer) in self.observers.get_mut().iter() {
            observer.leave();
        }
    }
}

/// What a cell holds, for showing it without knowing the type.
#[derive(Clone, Copy)]
struct Kind {
    name: &'static str,
    show: fn(&dyn Any, Option<&Open>) -> Node,
}

impl Kind {
    fn of<R: Reducer>() -> Self {
        Self {
            name: std::any::type_name::<R>(),
            show: |state, open| match state.downcast_ref::<Slot<R>>() {
                Some(cell) => match cell.try_borrow() {
                    Ok(state) => Node::shown(|at| state.show(at), open),
                    Err(_) => Node::busy("<being changed>"),
                },
                None => Node::busy("<unknown>"),
            },
        }
    }
}

/// One reducer as devtools see it.
#[derive(Clone, Debug, PartialEq)]
pub struct DescribedState {
    pub type_name: &'static str,
    /// Its preview; [`Scope::inspect_state`] opens it.
    pub state: Node,
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
            readers: RefCell::new(Vec::new()),
            observers: RefCell::new(Vec::new()),
        }
    }
}

impl Scope {
    /// The state of the reducer named `type_name` this scope holds, read as
    /// far as `open` reaches; `None` when it holds none by that name.
    pub fn inspect_state(&self, type_name: &str, open: &Open) -> Option<Node> {
        let data = self.data()?;
        let kinds = data.state.kinds.borrow();
        let (type_id, kind) = kinds.iter().find(|(_, kind)| kind.name == type_name)?;

        let cells = data.state.cells.borrow();
        let cell = cells.get(type_id)?;
        Some((kind.show)(&*cell.state, Some(open)))
    }

    /// Every reducer this scope holds whose type it has seen, previewed.
    pub fn describe_states(&self) -> Vec<DescribedState> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.state.cells.borrow();
        let kinds = data.state.kinds.borrow();
        let mut described: Vec<DescribedState> = cells
            .iter()
            .filter_map(|(type_id, cell)| {
                let kind = kinds.get(type_id)?;
                let claim = data.features.claimed(*type_id);
                Some(DescribedState {
                    type_name: kind.name,
                    state: (kind.show)(&*cell.state, None),
                    feature: claim.and_then(|claim| data.features.section_name(claim.section)),
                    declared: claim.and_then(|claim| claim.declared),
                })
            })
            .collect();
        described.sort_by_key(|state| state.type_name);
        described
    }

    /// `R`'s state here, created from `R::default()` on first read.
    pub fn state<R: Reducer>(&self) -> Rc<Slot<R>> {
        let (slot, made) = self.alive("reading a reducer").state.slot::<R>();
        if made {
            self.moved::<R>();
        }
        slot
    }

    fn moved<R: Reducer>(&self) {
        observer::moved(*self, std::any::type_name::<R>());
    }

    /// The reducers this scope holds whose type it has seen, told again to a
    /// tool that starts watching.
    pub(crate) fn held_states(&self) -> Vec<&'static str> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.state.cells.borrow();
        let kinds = data.state.kinds.borrow();

        let mut held: Vec<&'static str> = cells
            .keys()
            .filter_map(|type_id| Some(kinds.get(type_id)?.name))
            .collect();
        held.sort_unstable();
        held
    }

    pub fn peek<R: Reducer>(&self) -> Option<Rc<Slot<R>>> {
        let data = self.data()?;
        data.state.note_kind::<R>();
        let cells = data.state.cells.borrow();
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
    /// dropping its readers.
    pub fn seed<R: Reducer>(&self, state: R) {
        let data = self.alive("seeding a reducer");
        data.state.note_kind::<R>();
        let replaced = data
            .state
            .cells
            .borrow_mut()
            .insert(TypeId::of::<R>(), Cell::holding(Rc::new(Slot::new(Rc::new(state)))));
        drop(replaced);
        drop(data);

        self.moved::<R>();
    }

    /// Applies `msg` to `F`'s state now, and marks the cell so its readers
    /// are redrawn when the turn ends.
    ///
    /// The state is current the moment this returns; what waits is the
    /// redraw. Nothing a redraw does - navigating, dropping scopes, sending
    /// to another actor - happens on the stack of whoever pushed.
    pub fn push<R: Reducer>(&self, update: R::Update) {
        let data = self.alive("pushing an update");
        let carried: Option<Box<dyn Any>> = data
            .state
            .is_observed(TypeId::of::<R>())
            .then(|| Box::new(update.clone()) as Box<dyn Any>);
        let (state, _) = data.state.slot::<R>();
        drop(data);

        {
            let mut state = state.borrow_mut();
            Rc::make_mut(&mut state).reduce(update);
        }

        self.moved::<R>();
        crate::notify::mark(*self, TypeId::of::<R>(), carried);
    }

    /// Watches *what happened* to `F`, not merely that something did.
    ///
    /// A reader is redrawn because the cell changed, and reads the new state;
    /// an observer is handed the update itself, which is what tells a rename
    /// apart from a whole new list. Runs during the drain, before anything is
    /// redrawn, so state that depends on this one has settled by then.
    pub fn observe<R: Reducer>(&self, callback: impl Fn(&R::Update) + 'static) -> Subscription {
        let data = self.alive("observing a reducer");
        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        data.state.note_kind::<R>();
        {
            let mut cells = data.state.cells.borrow_mut();
            let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
            cell.observers.borrow_mut().push((
                id,
                Rc::new(Watcher::new(move |update: &dyn Any| {
                    if let Some(update) = update.downcast_ref::<R::Update>() {
                        callback(update);
                    }
                })),
            ));
        }

        let held = Rc::downgrade(&data);
        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(data) = held.upgrade() else { return };
                let observer = data.state.unobserve(TypeId::of::<R>(), id);
                drop(data);
                drop(observer);
            })),
        }
    }

    pub(crate) fn readers_of(&self, cell: TypeId) -> Vec<Rc<Watcher<dyn Any>>> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.state.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.readers.borrow().iter().map(|(_, reader)| reader.clone()).collect())
            .unwrap_or_default()
    }

    pub(crate) fn observers_of(&self, cell: TypeId) -> Vec<Rc<Watcher<dyn Fn(&dyn Any)>>> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.state.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.observers.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }

    /// See [`ReducerBinding::read_by`](crate::binding::ReducerBinding::read_by).
    pub(crate) fn read_by<R: Reducer, D: 'static>(&self, reader: D) -> Subscription {
        let data = self.alive("reading a reducer");

        match self.tree_drawing() {
            Some(drawing) if drawing.reader() == TypeId::of::<D>() => {}
            Some(_) => panic!(
                "{} is read by a `{}` in {self:?}, and its tree is drawn with readers of \
                 another type",
                std::any::type_name::<R>(),
                std::any::type_name::<D>()
            ),
            None => panic!(
                "{} is read by a `{}` in {self:?}, and nothing draws its tree: call \
                 `drawn_by` on it first",
                std::any::type_name::<R>(),
                std::any::type_name::<D>()
            ),
        }

        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        data.state.note_kind::<R>();
        {
            let mut cells = data.state.cells.borrow_mut();
            let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
            let reader: Rc<Watcher<dyn Any>> = Rc::new(Watcher::new(reader));
            cell.readers.borrow_mut().push((id, reader));
        }

        let held = Rc::downgrade(&data);
        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(data) = held.upgrade() else { return };
                let reader = data.state.unread(TypeId::of::<R>(), id);
                drop(data);
                drop(reader);
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
        data.state
            .cells
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
        let data = self.alive("restoring state");
        let mut replaced = Vec::new();
        let mut restored = Vec::new();
        {
            let mut cells = data.state.cells.borrow_mut();
            let kinds = data.state.kinds.borrow();
            for (type_id, state) in states {
                restored.extend(kinds.get(&type_id).map(|kind| kind.name));
                replaced.extend(cells.insert(type_id, Cell::holding(state)));
            }
        }
        drop(replaced);
        drop(data);

        for reducer in restored {
            observer::moved(*self, reducer);
        }
    }
}

impl States {
    fn note_kind<R: Reducer>(&self) {
        self.kinds
            .borrow_mut()
            .entry(TypeId::of::<R>())
            .or_insert_with(Kind::of::<R>);
    }

    /// `R`'s cell, and whether it was made just now.
    fn slot<R: Reducer>(&self) -> (Rc<Slot<R>>, bool) {
        self.note_kind::<R>();
        let mut cells = self.cells.borrow_mut();
        let made = !cells.contains_key(&TypeId::of::<R>());
        let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
        let slot = cell
            .state
            .clone()
            .downcast::<Slot<R>>()
            .expect("Scope cell type mismatch for this TypeId - unreachable, keyed by R");
        (slot, made)
    }

    fn unread(&self, cell: TypeId, id: u64) -> Option<Rc<Watcher<dyn Any>>> {
        let cells = self.cells.borrow();
        let mut readers = cells.get(&cell)?.readers.borrow_mut();
        let at = readers.iter().position(|(reader, _)| *reader == id)?;

        let (_, reader) = readers.remove(at);
        reader.leave();
        Some(reader)
    }

    fn unobserve(&self, cell: TypeId, id: u64) -> Option<Rc<Watcher<dyn Fn(&dyn Any)>>> {
        let cells = self.cells.borrow();
        let mut observers = cells.get(&cell)?.observers.borrow_mut();
        let at = observers.iter().position(|(observer, _)| *observer == id)?;

        let (_, observer) = observers.remove(at);
        observer.leave();
        Some(observer)
    }

    fn is_observed(&self, cell: TypeId) -> bool {
        self.cells
            .borrow()
            .get(&cell)
            .is_some_and(|cell| !cell.observers.borrow().is_empty())
    }
}
