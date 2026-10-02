use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::task::JoinHandle;

use crate::actor::Addr;
use crate::actor::event_bus::subscribe::BusSubscription;
use crate::actor::registry::{ActorSnapshot, Owner};
use crate::actor::shape::Declared;
use crate::actor::traits::ManagedActor;

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

/// Where a segment's state, features and resources live, from the moment it
/// is installed until it is removed.
///
/// A name for one, and `Copy`: the tree of scopes on this thread owns what each
/// scope holds, and nothing else does. Holding a `Scope` does not keep it
/// alive, so a scope goes exactly when [`remove`](Self::remove) is called on it
/// or on a scope above it - children first - and not whenever the last of
/// whoever happened to be holding it lets go.
///
/// A removed scope's name never comes back: the slot it lived in is reused
/// under a new generation, so an old `Scope` keeps naming the scope that is
/// gone, and [`is_alive`](Self::is_alive) says so.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    index: u32,
    generation: u32,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scope({}v{})", self.index, self.generation)
    }
}

/// Which of its parent's outlets a scope sits in. A layout has one today,
/// [`MAIN`].
pub type Outlet = &'static str;

/// The outlet a scope sits in unless it is said otherwise.
pub const MAIN: Outlet = "main";

#[derive(Default)]
struct ScopeData {
    cells: RefCell<HashMap<TypeId, Cell>>,
    /// Kept apart from `cells`: a cell restored from the state cache arrives
    /// erased, and learns its type again at the next typed access.
    kinds: RefCell<HashMap<TypeId, Kind>>,
    teardowns: RefCell<Vec<Box<dyn FnOnce()>>>,
    /// Features (identified by their `install` function's own type) that
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
    /// The window this scope is the root of. See [`Scope::set_window`].
    window: std::cell::Cell<Option<u64>>,
    /// The actors it holds, for devtools to read. See [`Scope::hold_actor`].
    actors: RefCell<Vec<HeldActor>>,
}

struct HeldActor {
    id: usize,
    type_name: &'static str,
    shape: crate::actor::shape::Shape,
    owner: Owner,
    snapshot: Box<dyn Fn() -> String>,
}

impl HeldActor {
    fn read(&self) -> ActorSnapshot {
        ActorSnapshot {
            id: self.id,
            type_name: self.type_name,
            shape: self.shape,
            owner: self.owner,
            state: (self.snapshot)(),
        }
    }
}

/// Tells devtools an actor is no longer listed, when its scope goes.
struct Unlisted {
    root: Option<u64>,
    id: usize,
}

impl Teardown for Unlisted {
    fn teardown(self) {
        crate::devtools::changed(|| crate::devtools::Change::ActorRemoved {
            root: self.root,
            id: self.id,
        });
    }
}

struct Node {
    generation: u32,
    data: Option<Rc<ScopeData>>,
    parent: Option<u32>,
    outlet: Outlet,
    children: Vec<u32>,
}

/// Every scope on this thread, and which sits under which.
#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    free: Vec<u32>,
}

impl Tree {
    fn insert(&mut self, parent: Option<u32>, outlet: Outlet) -> Scope {
        let data = Some(Rc::new(ScopeData::default()));
        let index = match self.free.pop() {
            Some(index) => {
                let node = &mut self.nodes[index as usize];
                node.data = data;
                node.parent = parent;
                node.outlet = outlet;
                index
            }
            None => {
                self.nodes.push(Node {
                    generation: 0,
                    data,
                    parent,
                    outlet,
                    children: Vec::new(),
                });
                (self.nodes.len() - 1) as u32
            }
        };

        if let Some(parent) = parent {
            self.nodes[parent as usize].children.push(index);
        }

        Scope {
            index,
            generation: self.nodes[index as usize].generation,
        }
    }

    fn node(&self, scope: Scope) -> Option<&Node> {
        self.nodes
            .get(scope.index as usize)
            .filter(|node| node.generation == scope.generation && node.data.is_some())
    }

    /// Takes `scope` and everything under it out of the tree, children
    /// before their parent and the newest child first, and hands back what
    /// they held in that order.
    fn detach(&mut self, scope: Scope) -> Vec<Rc<ScopeData>> {
        let Some(node) = self.node(scope) else {
            return Vec::new();
        };
        if let Some(parent) = node.parent {
            self.nodes[parent as usize].children.retain(|child| *child != scope.index);
        }

        let mut order = Vec::new();
        self.collect(scope.index, &mut order);

        let mut detached = Vec::with_capacity(order.len());
        for index in order {
            let node = &mut self.nodes[index as usize];
            node.generation = node.generation.wrapping_add(1);
            node.parent = None;
            node.children.clear();
            if let Some(data) = node.data.take() {
                detached.push(data);
            }
            self.free.push(index);
        }
        detached
    }

    fn collect(&self, index: u32, order: &mut Vec<u32>) {
        for child in self.nodes[index as usize].children.iter().rev() {
            self.collect(*child, order);
        }
        order.push(index);
    }
}

thread_local! {
    static TREE: RefCell<Tree> = RefCell::new(Tree::default());
}

/// Whether a scope is awake, for something it owns to ask on its own.
///
/// Held rather than the scope itself, so that a timer the scope owns can ask
/// from off the scope's own code paths.
#[derive(Clone)]
pub struct Awake(Rc<std::cell::Cell<bool>>);

impl Awake {
    /// Whether the scope is awake now.
    pub fn now(&self) -> bool {
        !self.0.get()
    }
}

/// Removes its scope when dropped: for whoever creates a root, or a child
/// that nothing else will remove.
#[must_use = "the scope is removed as soon as this is dropped"]
pub struct ScopeGuard(Scope);

impl ScopeGuard {
    pub fn scope(&self) -> Scope {
        self.0
    }
}

impl std::ops::Deref for ScopeGuard {
    type Target = Scope;

    fn deref(&self) -> &Scope {
        &self.0
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        self.0.remove();
    }
}

impl Scope {
    /// A scope with nothing above it: a window, a harness, a test.
    pub fn root() -> Scope {
        TREE.with(|tree| tree.borrow_mut().insert(None, MAIN))
    }

    /// A new scope under this one, removed when this one is.
    pub fn child(self) -> Scope {
        self.child_in(MAIN)
    }

    /// A new scope under this one, in `outlet`.
    pub fn child_in(self, outlet: Outlet) -> Scope {
        TREE.with(|tree| {
            let mut tree = tree.borrow_mut();
            assert!(
                tree.node(self).is_some(),
                "a child for {self:?}, which was removed"
            );
            tree.insert(Some(self.index), outlet)
        })
    }

    /// Removes this scope and every scope under it, now: children before
    /// their parent, and each one's resources in the order it took them.
    ///
    /// From the moment this is called, every `Scope` naming them is dead.
    /// Removing a scope that is already gone does nothing.
    pub fn remove(self) {
        let detached = TREE
            .try_with(|tree| tree.borrow_mut().detach(self))
            .unwrap_or_default();

        for data in detached {
            data.tear_down();
        }
    }

    /// Whether this scope is still there.
    pub fn is_alive(self) -> bool {
        TREE.try_with(|tree| tree.borrow().node(self).is_some())
            .unwrap_or(false)
    }

    /// The scope this one sits under, if it has one and it is still there.
    pub fn parent(self) -> Option<Scope> {
        TREE.try_with(|tree| {
            let tree = tree.borrow();
            let parent = tree.node(self)?.parent?;
            Some(Scope {
                index: parent,
                generation: tree.nodes[parent as usize].generation,
            })
        })
        .ok()
        .flatten()
    }

    /// Which of its parent's outlets this scope sits in.
    pub fn outlet(self) -> Option<Outlet> {
        TREE.try_with(|tree| tree.borrow().node(self).map(|node| node.outlet))
            .ok()
            .flatten()
    }

    /// The scopes above this one, the nearest first.
    pub fn ancestors(self) -> Vec<Scope> {
        std::iter::successors(self.parent(), |scope| scope.parent()).collect()
    }

    /// Where `R` is read from here: this scope if it claimed `R`, or the
    /// nearest one above that exports it.
    ///
    /// The two ends are asked different questions on purpose. A segment may
    /// read anything it claimed itself; what a scope above claimed is its own
    /// business unless it said otherwise in `Exports`.
    pub fn owner_of<R: 'static>(self) -> Option<Scope> {
        if self.claims::<R>() {
            return Some(self);
        }

        std::iter::successors(self.parent(), |scope| scope.parent())
            .find(|scope| scope.exports::<R>())
    }

    /// Says this scope is the root of window `id`, as `RootId::get` numbers
    /// it: what devtools are told the actors under it belong to.
    pub fn set_window(self, id: u64) {
        if let Some(data) = self.data() {
            data.window.set(Some(id));
        }
    }

    /// The window this scope is under, if it is under one.
    pub fn window(self) -> Option<u64> {
        std::iter::successors(Some(self), |scope| scope.parent())
            .find_map(|scope| scope.data()?.window.get())
    }

    /// Lists `addr` among the actors this scope holds, for devtools, until
    /// the scope is removed - `feature` created it, and it drives `drives`
    /// when it was made to.
    ///
    /// Only the listing: what ends the actor is still whoever owns it, which
    /// for an actor of a segment is [`own`](Self::own) on this same scope.
    pub fn hold_actor<A: ManagedActor + std::fmt::Debug>(
        self,
        addr: &Addr<A>,
        feature: Option<&'static str>,
        drives: Option<&'static str>,
    ) {
        let Some(data) = self.data() else { return };
        let id = addr.id();
        let type_name = crate::actor::short_type_name::<A>();
        let owner = Owner {
            scope: Some(self.key()),
            feature,
            drives,
        };

        let held = addr.clone();
        data.actors.borrow_mut().push(HeldActor {
            id,
            type_name,
            shape: A::SHAPE,
            owner,
            snapshot: Box::new(move || held.debug_snapshot()),
        });
        drop(data);

        let root = self.window();
        crate::devtools::changed(|| crate::devtools::Change::ActorAdded {
            root,
            id,
            type_name,
            owner,
        });
        self.own(Unlisted { root, id });
    }

    /// The actors this scope and every scope under it hold, read now.
    pub fn actors(self) -> Vec<ActorSnapshot> {
        self.subtree()
            .into_iter()
            .filter_map(Scope::data)
            .flat_map(|data| data.actors.borrow().iter().map(HeldActor::read).collect::<Vec<_>>())
            .collect()
    }

    /// The actor `id`, if this scope or one under it holds it, read now; the
    /// others are not read.
    pub fn actor(self, id: usize) -> Option<ActorSnapshot> {
        self.subtree().into_iter().filter_map(Scope::data).find_map(|data| {
            data.actors
                .borrow()
                .iter()
                .find(|actor| actor.id == id)
                .map(HeldActor::read)
        })
    }

    /// This scope and every scope under it, children first.
    fn subtree(self) -> Vec<Scope> {
        TREE.try_with(|tree| {
            let tree = tree.borrow();
            if tree.node(self).is_none() {
                return Vec::new();
            }

            let mut order = Vec::new();
            tree.collect(self.index, &mut order);
            order
                .into_iter()
                .map(|index| Scope {
                    index,
                    generation: tree.nodes[index as usize].generation,
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Removes this scope when the guard is dropped.
    pub fn guard(self) -> ScopeGuard {
        ScopeGuard(self)
    }

    /// Identifies this scope, and never another one.
    pub fn key(self) -> usize {
        ((self.generation as usize) << 32) | self.index as usize
    }

    fn data(self) -> Option<Rc<ScopeData>> {
        TREE.try_with(|tree| tree.borrow().node(self).and_then(|node| node.data.clone()))
            .ok()
            .flatten()
    }

    fn installing(self, doing: &str) -> Rc<ScopeData> {
        self.data()
            .unwrap_or_else(|| panic!("{doing} in {self:?}, which was removed"))
    }

    /// Marks feature `F` (its `install` function, used purely as a type
    /// identity) as owned by this scope. `F` must not already be marked
    /// here - two different call sites both claiming ownership of the same
    /// feature in the same scope is a setup bug, not something to merge
    /// silently.
    pub fn mark_feature_installed<F: 'static>(self) {
        let data = self.installing("installing a feature");
        let newly_inserted = data.installed_features.borrow_mut().insert(TypeId::of::<F>());
        assert!(
            newly_inserted,
            "feature already installed in this scope - install() called twice for the same feature"
        );
    }

    /// Whether anything in this scope has claimed `R`.
    ///
    /// What tells an export that was earned from one that was only declared.
    pub fn claims<R: 'static>(self) -> bool {
        self.data()
            .is_some_and(|data| data.owners.borrow().contains_key(&TypeId::of::<R>()))
    }

    pub fn note_reducer_owner<R: 'static>(self) {
        let data = self.installing("claiming a reducer");
        data.installed_features.borrow_mut().insert(TypeId::of::<R>());
        let section = data.current_section();
        data.owners.borrow_mut().entry(TypeId::of::<R>()).or_insert(section);
    }

    /// Notes where reducer `R` was claimed, the first claim winning.
    pub fn note_reducer_declared<R: 'static>(self, declared: Declared) {
        let data = self.installing("claiming a reducer");
        data.declarations.borrow_mut().entry(TypeId::of::<R>()).or_insert(declared);
    }

    /// The manifest directory of the feature installing now, for a claim that
    /// only knows the file the compiler gave it.
    pub fn current_crate_dir(self) -> Option<&'static str> {
        let data = self.data()?;
        let section = data.current_section();
        let declarations = data.section_declarations.borrow();
        declarations.get(section).copied().flatten().map(|declared| declared.crate_dir)
    }

    /// Opens a section for the feature `name` about to install. Returns its
    /// index.
    pub fn open_section(self, name: &'static str, declared: Option<Declared>) -> usize {
        let data = self.installing("installing a feature");
        let mut sections = data.sections.borrow_mut();
        if sections.is_empty() {
            sections.push(HashMap::new());
        }
        sections.push(HashMap::new());
        let index = sections.len() - 1;
        let mut names = data.section_names.borrow_mut();
        names.resize(index + 1, None);
        names[index] = Some(name);

        let mut declarations = data.section_declarations.borrow_mut();
        declarations.resize(index + 1, None);
        declarations[index] = declared;

        data.installing.borrow_mut().push(index);
        index
    }

    /// The feature a section belongs to; `None` for the segment's own.
    pub fn section_name(self, section: usize) -> Option<&'static str> {
        self.data()?.section_name(section)
    }

    /// Every feature installed here, in the order they were.
    pub fn features(self) -> Vec<Installed> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let declarations = data.section_declarations.borrow();

        data.section_names
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
    pub fn current_feature(self) -> Option<&'static str> {
        let data = self.data()?;
        data.section_name(data.current_section())
    }

    /// Notes that whatever is installing listens to `event` on `bus` - through
    /// `actor` when an actor does the listening.
    pub fn note_listener(
        self,
        event: &'static str,
        actor: Option<&'static str>,
        bus: crate::trace::Bus,
    ) {
        let Some(data) = self.data() else { return };
        let feature = data.section_name(data.current_section());
        data.listeners.borrow_mut().push(Listener {
            event,
            actor,
            bus,
            feature,
        });
    }

    pub fn listeners(self) -> Vec<Listener> {
        self.data()
            .map(|data| data.listeners.borrow().clone())
            .unwrap_or_default()
    }

    pub fn close_section(self) {
        if let Some(data) = self.data() {
            data.installing.borrow_mut().pop();
        }
    }

    /// The section being installed, or the segment's own when none is.
    pub fn current_section(self) -> usize {
        self.data().map_or(0, |data| data.current_section())
    }

    /// Which section owns `R` - the instance whose dispatcher a reader of `R`
    /// should be handed.
    pub fn section_of<R: 'static>(self) -> usize {
        self.data()
            .and_then(|data| data.owners.borrow().get(&TypeId::of::<R>()).copied())
            .unwrap_or(0)
    }

    /// Whether feature `F` was marked installed in *this exact* scope.
    pub fn has_feature<F: 'static>(self) -> bool {
        self.data()
            .is_some_and(|data| data.installed_features.borrow().contains(&TypeId::of::<F>()))
    }

    /// Marks `R` as readable from segments below this one.
    ///
    /// Called by `cx.install::<F>()` for everything in `F::Exports`, and by
    /// nothing else. A reducer a feature claimed but did not export stays
    /// visible to the feature itself and invisible from below - which is the
    /// whole difference between a feature and a folder.
    pub fn note_export<R: 'static>(self) {
        let data = self.installing("exporting a reducer");
        data.exports.borrow_mut().insert(TypeId::of::<R>());
    }

    /// Whether `R` is readable from below this scope.
    pub fn exports<R: 'static>(self) -> bool {
        self.data()
            .is_some_and(|data| data.exports.borrow().contains(&TypeId::of::<R>()))
    }

    /// Every reducer this scope holds whose type it has seen, printed.
    pub fn describe_states(self) -> Vec<DescribedState> {
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
    pub fn state<R: Reducer>(self) -> Rc<Slot<R>> {
        match self.data() {
            Some(data) => data.state::<R>(),
            None => Rc::new(Slot::new(Rc::new(R::default()))),
        }
    }

    pub fn peek<R: Reducer>(self) -> Option<Rc<Slot<R>>> {
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
    pub fn seed<R: Reducer>(self, state: R) {
        let data = self.installing("seeding a reducer");
        data.note_kind::<R>();
        let replaced = data
            .cells
            .borrow_mut()
            .insert(TypeId::of::<R>(), Cell::holding(Rc::new(Slot::new(Rc::new(state)))));
        drop(replaced);
    }

    /// Says that this scope answers `M`, and how.
    ///
    /// Keyed by the action, not by whoever answers it - which is what keeps
    /// the answerer out of every signature the UI touches. `actor!` calls this
    /// for each handler it lists; a domain that runs on tasks, or a channel,
    /// or a plain closure over a `RefCell`, calls it itself.
    pub fn answers<M: 'static>(self, answer: impl Fn(M) + 'static) {
        let data = self.installing("answering an action");
        let answer: Rc<dyn Fn(M)> = Rc::new(answer);
        let section = data.current_section();

        let mut sections = data.sections.borrow_mut();
        while sections.len() <= section {
            sections.push(HashMap::new());
        }
        sections[section].insert(TypeId::of::<M>(), Rc::new(answer) as Rc<dyn Any>);
    }

    /// What answers `M` in one section of this scope, if anything does.
    pub fn answerer<M: 'static>(self, section: usize) -> Option<Rc<dyn Fn(M)>> {
        self.data()?.answerer::<M>(section)
    }

    /// What answers `M` anywhere in this scope - the first feature that does,
    /// in the order they installed. For a sender that knows the action and
    /// not which state it was reading.
    pub fn first_answerer<M: 'static>(self) -> Option<Rc<dyn Fn(M)>> {
        let data = self.data()?;
        let sections = data.sections.borrow().len();
        (0..sections).find_map(|section| data.answerer::<M>(section))
    }

    /// Applies `msg` to `F`'s state now, and marks the cell so its listeners
    /// run at the next [`notify::drain`](crate::notify::drain).
    ///
    /// The state is current the moment this returns; what waits is the
    /// redraw. Nothing a listener does - navigating, dropping scopes, sending
    /// to another actor - happens on the stack of whoever pushed. A no-op once
    /// the scope is removed.
    pub fn push<R: Reducer>(self, update: R::Update) {
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

        crate::notify::mark(self, TypeId::of::<R>(), carried);
    }

    /// Watches *what happened* to `F`, not merely that something did.
    ///
    /// A listener from [`subscribe`](Self::subscribe) learns the cell changed
    /// and reads the new state; an observer is handed the update itself, which
    /// is what tells a rename apart from a whole new list. Runs during the
    /// drain, before any listener, so state that depends on this one has
    /// settled by the time anything draws.
    pub fn observe<R: Reducer>(self, callback: impl Fn(&R::Update) + 'static) -> Subscription {
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

        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(data) = self.data() else { return };
                if let Some(cell) = data.cells.borrow_mut().get_mut(&TypeId::of::<R>()) {
                    cell.observers.borrow_mut().retain(|(oid, _)| *oid != id);
                }
            })),
        }
    }

    pub(crate) fn listeners_of(self, cell: TypeId) -> Vec<Rc<dyn Fn()>> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.listeners.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }

    pub(crate) fn observers_of(self, cell: TypeId) -> Vec<Rc<dyn Fn(&dyn Any)>> {
        let Some(data) = self.data() else {
            return Vec::new();
        };
        let cells = data.cells.borrow();
        cells
            .get(&cell)
            .map(|cell| cell.observers.borrow().iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }

    pub fn subscribe<R: Reducer>(self, callback: impl Fn() + 'static) -> Subscription {
        let Some(data) = self.data() else {
            return Subscription::inert();
        };
        let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        {
            let mut cells = data.cells.borrow_mut();
            let cell = cells.entry(TypeId::of::<R>()).or_insert_with(Cell::empty::<R>);
            cell.listeners.borrow_mut().push((id, Rc::new(callback)));
        }

        Subscription {
            unsubscribe: Some(Box::new(move || {
                let Some(data) = self.data() else { return };
                if let Some(cell) = data.cells.borrow_mut().get_mut(&TypeId::of::<R>()) {
                    cell.listeners.borrow_mut().retain(|(lid, _)| *lid != id);
                }
            })),
        }
    }

    /// A handle to `R`'s state and actions in this scope, for code that reads
    /// or watches them without being a UI hook. See [`crate::binding`].
    pub fn binding<R: Reducer>(self) -> crate::binding::ReducerBinding<R> {
        crate::binding::ReducerBinding::new(self)
    }

    pub fn own_subscription(self, subscription: BusSubscription) {
        self.own(DropGuard(subscription));
    }

    /// Returns a shallow clone of every reducer state currently held by this
    /// scope, keyed by the reducer's `TypeId`. Used by the router to cache
    /// page state in memory while the page is not mounted.
    pub fn snapshot_states(self) -> HashMap<TypeId, Rc<dyn Any>> {
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
    pub fn restore_states(self, states: HashMap<TypeId, Rc<dyn Any>>) {
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

    /// Binds any [`Teardown`] resource to this scope's lifetime: it is torn
    /// down when the scope is - at once, if the scope is already gone.
    pub fn own<R: Teardown>(self, resource: R) {
        match self.data() {
            Some(data) => data
                .teardowns
                .borrow_mut()
                .push(Box::new(move || resource.teardown())),
            None => resource.teardown(),
        }
    }

    /// Asked before this scope is torn down by a navigation.
    ///
    /// Registered during `install`, which is the whole reason leaving and
    /// entering are declared in different places: on the way out the scope
    /// exists, so the guard can read its own state - which is what "unsaved
    /// changes" is. On the way in there is nothing to read yet.
    pub fn on_leave(self, guard: impl Fn() -> crate::guard::Verdict + 'static) {
        let data = self.installing("guarding a leave");
        data.leave_guards.borrow_mut().push(Rc::new(guard));
    }

    /// The guards to ask, in the order they were registered.
    ///
    /// Cloned out rather than borrowed: a guard is free to touch this scope,
    /// and the caller runs them while deciding.
    pub fn leave_guards(self) -> Vec<Rc<dyn Fn() -> crate::guard::Verdict>> {
        self.data()
            .map(|data| data.leave_guards.borrow().clone())
            .unwrap_or_default()
    }

    /// Puts this scope to sleep: kept, with its state and actors, but deaf.
    ///
    /// What it owns stops hearing anything while it sleeps - its timers skip
    /// their ticks, its actors and callbacks are not told what the buses
    /// carry - and nothing is queued for later: what happened meanwhile is
    /// missed. A router does this to a `keep` segment it leaves.
    pub fn sleep(self) {
        if let Some(data) = self.data() {
            data.asleep.set(true);
        }
    }

    /// Wakes this scope, then runs what asked to hear of it, in the order it
    /// asked.
    pub fn wake(self) {
        let Some(data) = self.data() else { return };
        data.asleep.set(false);

        let hooks = data.wake_hooks.borrow().clone();
        drop(data);
        for hook in hooks {
            hook();
        }
    }

    /// Whether this scope is there and awake. A removed one is neither.
    pub fn is_awake(self) -> bool {
        self.data().is_some_and(|data| !data.asleep.get())
    }

    /// Whether this scope is awake, for what it owns to ask later.
    pub fn awake(self) -> Awake {
        match self.data() {
            Some(data) => Awake(data.asleep.clone()),
            None => Awake(Rc::new(std::cell::Cell::new(true))),
        }
    }

    /// Runs `hook` every time this scope wakes: for a feature to catch up on
    /// what it missed while it slept.
    pub fn on_wake(self, hook: impl Fn() + 'static) {
        let data = self.installing("waiting for a wake");
        data.wake_hooks.borrow_mut().push(Rc::new(hook));
    }
}

impl ScopeData {
    fn section_name(&self, section: usize) -> Option<&'static str> {
        self.section_names.borrow().get(section).copied().flatten()
    }

    fn current_section(&self) -> usize {
        self.installing.borrow().last().copied().unwrap_or(0)
    }

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

    fn answerer<M: 'static>(&self, section: usize) -> Option<Rc<dyn Fn(M)>> {
        let sections = self.sections.borrow();
        let answer = sections.get(section)?.get(&TypeId::of::<M>())?.clone();
        answer.downcast::<Rc<dyn Fn(M)>>().ok().map(|a| (*a).clone())
    }

    fn is_observed(&self, cell: TypeId) -> bool {
        self.cells
            .borrow()
            .get(&cell)
            .is_some_and(|cell| !cell.observers.borrow().is_empty())
    }

    /// Runs what this scope took on, in the order it took it.
    fn tear_down(&self) {
        let teardowns = std::mem::take(&mut *self.teardowns.borrow_mut());
        for teardown in teardowns {
            teardown();
        }
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

/// The application's own scope on this thread: a root nothing removes, for
/// state that belongs to no window.
pub struct GlobalScope;

impl GlobalScope {
    pub fn instance() -> Scope {
        thread_local! {
            static SCOPE: Scope = Scope::root();
        }
        SCOPE.with(|scope| *scope)
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

    struct Said(&'static str, Rc<RefCell<Vec<&'static str>>>);

    impl Teardown for Said {
        fn teardown(self) {
            self.1.borrow_mut().push(self.0);
        }
    }

    #[test]
    fn removing_a_scope_tears_down_what_is_under_it_first() {
        let said = Rc::new(RefCell::new(Vec::new()));
        let root = Scope::root();
        let older = root.child();
        let below = older.child();
        let newer = root.child();

        for (scope, name) in [(root, "root"), (older, "older"), (below, "below"), (newer, "newer")] {
            scope.own(Said(name, said.clone()));
        }
        root.remove();

        assert_eq!(*said.borrow(), ["newer", "below", "older", "root"]);
    }

    #[test]
    fn a_removed_scope_stays_gone_and_its_slot_names_another_one() {
        let root = Scope::root();
        let gone = root.child();
        gone.remove();
        let next = root.child();

        assert!(!gone.is_alive());
        assert!(next.is_alive());
        assert_ne!(gone, next);
        assert_ne!(gone.key(), next.key());
        assert_eq!(next.parent(), Some(root));
        root.remove();
    }

    #[test]
    fn a_scope_goes_when_it_is_removed_however_many_name_it() {
        let said = Rc::new(RefCell::new(Vec::new()));
        let scope = Scope::root();
        let named_elsewhere = scope;
        scope.own(Said("torn down", said.clone()));

        scope.remove();

        assert_eq!(*said.borrow(), ["torn down"]);
        assert!(!named_elsewhere.is_alive());
    }

    #[test]
    fn what_a_removed_scope_is_handed_is_torn_down_at_once() {
        let said = Rc::new(RefCell::new(Vec::new()));
        let scope = Scope::root();
        scope.remove();

        scope.own(Said("at once", said.clone()));

        assert_eq!(*said.borrow(), ["at once"]);
    }

    #[derive(Debug)]
    struct Ticker(u32);

    struct Tick;

    guinea_macros::actor! {
        Ticker {
            handlers {
                Tick
            }
        }
    }

    impl crate::actor::Handler<Tick> for Ticker {
        fn handle(&mut self, _: Tick, _cx: crate::actor::Cx<Self, Tick>) {
            self.0 += 1;
        }
    }

    fn ticker() -> Addr<Ticker> {
        Addr::new_managed_scoped(
            Ticker(0),
            crate::actor::UiThreadToken::dangerously_create_token_unchecked(),
        )
    }

    fn watched(run: impl FnOnce()) -> Vec<crate::devtools::Change> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        crate::devtools::watch(move |change| sink.borrow_mut().push(change.clone()));

        run();
        crate::devtools::stop_watching();

        seen.take()
    }

    #[test]
    fn devtools_hear_an_actor_a_scope_holds_come_and_go_and_whose_it_is() {
        use crate::devtools::Change;

        let window = Scope::root();
        window.set_window(7);
        let page = window.child();
        let addr = ticker();
        let id = addr.id();

        let seen = watched(|| {
            page.hold_actor(&addr, Some("app::Clock"), Some("Time"));
            window.remove();
        });

        assert_eq!(
            seen,
            [
                Change::ActorAdded {
                    root: Some(7),
                    id,
                    type_name: crate::actor::short_type_name::<Ticker>(),
                    owner: Owner {
                        scope: Some(page.key()),
                        feature: Some("app::Clock"),
                        drives: Some("Time"),
                    },
                },
                Change::ActorRemoved { root: Some(7), id },
            ]
        );
    }

    #[test]
    fn a_window_reads_the_actors_under_it_and_one_by_id_without_the_rest() {
        let window = Scope::root();
        let page = window.child();
        let (bumped, other) = (ticker(), ticker());
        window.hold_actor(&other, None, None);
        page.hold_actor(&bumped, None, None);
        bumped.send(Tick);

        let mut held: Vec<usize> = window.actors().iter().map(|actor| actor.id).collect();
        held.sort_unstable();
        let mut expected = vec![bumped.id(), other.id()];
        expected.sort_unstable();
        assert_eq!(held, expected);
        assert_eq!(page.actors().len(), 1, "a page reads only what is under it");

        let read = window.actor(bumped.id()).map(|actor| actor.state);
        assert_eq!(read, Some(format!("{:#?}", Ticker(1))));
        assert!(window.actor(usize::MAX).is_none());

        window.remove();
        assert!(window.actors().is_empty(), "what a removed scope held is not listed");
    }

    #[test]
    fn a_reducer_is_read_where_it_was_claimed_or_from_the_nearest_export_above() {
        #[derive(Clone, Default, Debug)]
        struct Hidden;

        impl Reducer for Hidden {
            type Update = ();

            fn reduce(&mut self, _: ()) {}
        }

        let root = Scope::root();
        let layout = root.child();
        let page = layout.child();
        root.note_reducer_owner::<Counter>();
        root.note_export::<Counter>();
        layout.note_reducer_owner::<Counter>();
        layout.note_export::<Counter>();
        layout.note_reducer_owner::<Hidden>();
        page.note_reducer_owner::<Hidden>();

        assert_eq!(page.owner_of::<Counter>(), Some(layout), "the nearest export wins");
        assert_eq!(page.owner_of::<Hidden>(), Some(page), "what it claimed is its own");
        assert_eq!(layout.child().owner_of::<Hidden>(), None, "claimed above is not exported");
        assert_eq!(root.owner_of::<Counter>(), Some(root));
        root.remove();
    }

    #[test]
    fn the_global_scope_is_one_scope_with_nothing_above_it() {
        GlobalScope::instance().push::<Counter>(CounterMsg::Set(3));

        assert_eq!(GlobalScope::instance(), GlobalScope::instance());
        assert_eq!(GlobalScope::instance().parent(), None);
        assert_eq!(GlobalScope::instance().state::<Counter>().borrow().value, 3);
    }

    #[test]
    fn a_described_state_names_the_feature_that_claimed_it() {
        #[derive(Clone, Default, Debug)]
        struct Loose;

        impl Reducer for Loose {
            type Update = ();

            fn reduce(&mut self, _: ()) {}
        }

        let scope = Scope::root();
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
        scope.remove();
    }

    #[test]
    fn state_survives_unmount_remount_within_a_live_store() {
        let store = Scope::root();

        let first_read = store.state::<Counter>();
        assert_eq!(first_read.borrow().value, 0);
        store.push::<Counter>(CounterMsg::Set(42));

        drop(first_read);

        let second_read = store.state::<Counter>();
        assert_eq!(second_read.borrow().value, 42);
        store.remove();
    }

    #[test]
    fn push_notifies_subscribers_and_unsubscribe_stops_it() {
        let store = Scope::root();
        let seen = Rc::new(RefCell::new(Vec::new()));

        let seen_for_sub = seen.clone();
        let sub = store.subscribe::<Counter>(move || {
            seen_for_sub.borrow_mut().push(store.state::<Counter>().borrow().value);
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
        store.remove();
    }

    #[tokio::test]
    async fn removing_the_store_aborts_owned_tasks() {
        let ran_to_completion = Arc::new(AtomicBool::new(false));
        let flag = ran_to_completion.clone();

        let store = Scope::root();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            flag.store(true, Ordering::SeqCst);
        });
        store.own(handle);

        store.remove();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !ran_to_completion.load(Ordering::SeqCst),
            "task should have been aborted when its owning scope was removed"
        );
    }

    #[test]
    fn own_actor_disposes_the_registry_entry_on_removal() {
        let token = crate::actor::UiThreadToken::dangerously_create_token_unchecked();
        let addr = Addr::new_scoped((), token);
        let counter = addr.strong_count_ptr();

        let store = Scope::root();
        store.own(addr.clone());
        drop(addr);

        assert!(
            Rc::strong_count(&counter) > 1,
            "REGISTRY should still hold the actor alive while its Scope is alive"
        );

        store.remove();

        assert_eq!(
            Rc::strong_count(&counter),
            1,
            "removing the Scope should dispose the REGISTRY entry, \
             leaving only this test's own counter handle"
        );
    }
}
