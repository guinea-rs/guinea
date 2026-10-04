use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::actor::shape::Declared;

mod actors;
mod features;
mod lifetime;
mod state;
mod tree;

#[cfg(test)]
mod tests;

pub use features::{Installed, Listener};
pub use lifetime::{Awake, DropGuard, Teardown};
pub use state::{DescribedState, Reducer, Slot, StateHandle, Subscription};
pub use tree::{ScopeGuard, ScopeTree};

use actors::HeldActor;
use state::{Cell, Kind};
use tree::{Tree, tree};

/// Where a segment's state, features and resources live, from the moment it
/// is installed until it is removed.
///
/// A name for one, and `Copy`: the [`ScopeTree`] it is in owns what each scope
/// holds, and nothing else does. Holding a `Scope` does not keep it alive, so
/// a scope goes exactly when [`remove`](Self::remove) is called on it or on a
/// scope above it - children first - or when its tree's owner lets go of the
/// tree, and not whenever the last of whoever happened to be holding it lets
/// go.
///
/// A removed scope's name never comes back: every scope is numbered once, so
/// an old `Scope` keeps naming the scope that is gone, and
/// [`is_alive`](Self::is_alive) says so.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    tree: u32,
    index: u32,
    serial: u64,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scope({})", self.serial)
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

impl Scope {
    /// A new scope under this one, removed when this one is.
    pub fn child(&self) -> Scope {
        self.child_in(MAIN)
    }

    /// A new scope under this one, in `outlet`.
    pub fn child_in(&self, outlet: Outlet) -> Scope {
        let tree =
            tree(self.tree).unwrap_or_else(|| panic!("a child for {self:?}, whose tree is gone"));
        let mut nodes = tree.borrow_mut();
        assert!(
            nodes.node(self.index, self.serial).is_some(),
            "a child for {self:?}, which was removed"
        );
        let (index, serial) = nodes.insert(Some(self.index), outlet);

        Scope {
            tree: self.tree,
            index,
            serial,
        }
    }

    /// Removes this scope and every scope under it, now: children before
    /// their parent, and each one's resources the last first.
    ///
    /// From the moment this is called, every `Scope` naming them is dead.
    /// Removing a scope that is already gone does nothing.
    pub fn remove(&self) {
        let Some(tree) = tree(self.tree) else {
            return;
        };
        let detached = tree.borrow_mut().detach(self.index, self.serial);
        drop(tree);

        for data in detached {
            data.tear_down();
        }
    }

    /// Whether this scope is still there.
    pub fn is_alive(&self) -> bool {
        self.read(|tree| tree.node(self.index, self.serial).is_some())
            .unwrap_or(false)
    }

    /// The scope this one sits under, if it has one and it is still there.
    pub fn parent(&self) -> Option<Scope> {
        self.read(|tree| {
            let parent = tree.node(self.index, self.serial)?.parent?;
            Some(self.named(tree, parent))
        })
        .flatten()
    }

    /// Which of its parent's outlets this scope sits in.
    pub fn outlet(&self) -> Option<Outlet> {
        self.read(|tree| tree.node(self.index, self.serial).map(|node| node.outlet))
            .flatten()
    }

    /// The scopes above this one, the nearest first.
    pub fn ancestors(&self) -> Vec<Scope> {
        std::iter::successors(self.parent(), Scope::parent).collect()
    }

    /// The scopes right under this one, in `outlet`, oldest first.
    pub fn children_in(&self, outlet: Outlet) -> Vec<Scope> {
        self.read(|tree| {
            let Some(node) = tree.node(self.index, self.serial) else {
                return Vec::new();
            };
            node.children
                .iter()
                .filter(|&&child| tree.nodes[child as usize].outlet == outlet)
                .map(|&child| self.named(tree, child))
                .collect()
        })
        .unwrap_or_default()
    }

    /// The scopes right under this one in any outlet but [`MAIN`], oldest
    /// first: what a layout mounts beside its chain.
    pub fn beside(&self) -> Vec<Scope> {
        self.read(|tree| {
            let Some(node) = tree.node(self.index, self.serial) else {
                return Vec::new();
            };
            node.children
                .iter()
                .filter(|&&child| tree.nodes[child as usize].outlet != MAIN)
                .map(|&child| self.named(tree, child))
                .collect()
        })
        .unwrap_or_default()
    }

    fn read<T>(&self, read: impl FnOnce(&Tree) -> T) -> Option<T> {
        let tree = tree(self.tree)?;
        let nodes = tree.borrow();
        Some(read(&nodes))
    }

    fn named(&self, tree: &Tree, index: u32) -> Scope {
        Scope {
            tree: self.tree,
            index,
            serial: tree.nodes[index as usize].serial,
        }
    }

    /// Where `R` is read from here: this scope if it claimed `R`, or the
    /// nearest one above that exports it.
    ///
    /// The two ends are asked different questions on purpose. A segment may
    /// read anything it claimed itself; what a scope above claimed is its own
    /// business unless it said otherwise in `Exports`.
    pub fn owner_of<R: 'static>(&self) -> Option<Scope> {
        if self.claims::<R>() {
            return Some(*self);
        }

        std::iter::successors(self.parent(), Scope::parent).find(|scope| scope.exports::<R>())
    }

    /// This scope and every scope under it, children first.
    fn subtree(&self) -> Vec<Scope> {
        self.read(|tree| {
            if tree.node(self.index, self.serial).is_none() {
                return Vec::new();
            }

            let mut order = Vec::new();
            tree.collect(self.index, &mut order);
            order
                .into_iter()
                .map(|index| self.named(tree, index))
                .collect()
        })
        .unwrap_or_default()
    }

    /// Removes this scope when the guard is dropped.
    pub fn guard(&self) -> ScopeGuard {
        ScopeGuard(*self)
    }

    /// Identifies this scope, and never another one.
    pub fn key(&self) -> usize {
        self.serial as usize
    }

    fn data(&self) -> Option<Rc<ScopeData>> {
        self.read(|tree| {
            tree.node(self.index, self.serial)
                .and_then(|node| node.data.clone())
        })
        .flatten()
    }

    fn installing(&self, doing: &str) -> Rc<ScopeData> {
        self.data()
            .unwrap_or_else(|| panic!("{doing} in {self:?}, which was removed"))
    }
}

impl ScopeData {
    fn section_name(&self, section: usize) -> Option<&'static str> {
        self.section_names.borrow().get(section).copied().flatten()
    }

    fn current_section(&self) -> usize {
        self.installing.borrow().last().copied().unwrap_or(0)
    }
}
