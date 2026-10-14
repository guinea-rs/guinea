use std::rc::Rc;

use crate::actor::event_bus::EventBus;

mod actors;
mod features;
mod feeds;
mod held;
mod kept;
mod lifetime;
mod services;
mod state;
mod tree;

#[cfg(test)]
mod tests;

pub use features::{Installed, Listener, Section};
pub use lifetime::Awake;
pub use state::{DescribedState, Reducer, Slot, StateHandle, Subscription};
pub use tree::{ScopeGuard, ScopeTree};

pub(crate) use held::next_key;
use tree::{Node, NodeKey, Tree, TreeKey, tree};

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
/// A removed scope's name never comes back: its keys find nothing once it is
/// gone, even after another scope or another tree takes its place, so an old
/// `Scope` keeps naming the scope that is gone, and
/// [`is_alive`](Self::is_alive) says so.
///
/// It is its own key: equal, and hashed the same, only to itself.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    tree: TreeKey,
    node: NodeKey,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use slotmap::Key;
        write!(f, "Scope({:?}/{:?})", self.tree.data(), self.node.data())
    }
}

impl Scope {
    /// A new scope under this one, removed when this one is.
    pub fn child(&self) -> Scope {
        let tree =
            tree(self.tree).unwrap_or_else(|| panic!("a child for {self:?}, whose tree is gone"));
        let mut tree = tree.borrow_mut();
        assert!(
            tree.nodes.contains_key(self.node),
            "a child for {self:?}, which was removed"
        );
        let node = tree.child_of(self.node);

        Scope {
            tree: self.tree,
            node,
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
        let detached = tree.borrow_mut().detach(self.node);
        drop(tree);

        let Some(detached) = detached else {
            return;
        };
        for data in detached {
            data.held.let_go();
        }
    }

    /// Whether this scope is still there.
    pub fn is_alive(&self) -> bool {
        self.read(|tree| tree.nodes.contains_key(self.node))
            .unwrap_or(false)
    }

    /// The scope this one sits under, if it has one and it is still there.
    pub fn parent(&self) -> Option<Scope> {
        self.read(|tree| self.node_in(tree)?.parent.map(|parent| self.named(parent)))
            .flatten()
    }

    /// The scopes above this one, the nearest first.
    pub fn ancestors(&self) -> Vec<Scope> {
        std::iter::successors(self.parent(), Scope::parent).collect()
    }

    fn read<T>(&self, read: impl FnOnce(&Tree) -> T) -> Option<T> {
        let tree = tree(self.tree)?;
        let tree = tree.borrow();
        Some(read(&tree))
    }

    fn node_in<'t>(&self, tree: &'t Tree) -> Option<&'t Node> {
        tree.nodes.get(self.node)
    }

    fn named(&self, node: NodeKey) -> Scope {
        Scope {
            tree: self.tree,
            node,
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
    pub fn subtree(&self) -> Vec<Scope> {
        self.read(|tree| {
            if self.node_in(tree).is_none() {
                return Vec::new();
            }

            let mut order = Vec::new();
            tree.collect(self.node, &mut order);
            order.into_iter().map(|node| self.named(node)).collect()
        })
        .unwrap_or_default()
    }

    /// Removes this scope when the guard is dropped.
    pub fn guard(&self) -> ScopeGuard {
        ScopeGuard(*self)
    }

    /// The bus of this scope's tree: every scope in it publishes there and
    /// hears there.
    ///
    /// # Panics
    ///
    /// When this scope was removed: what it would subscribe would outlive it.
    pub fn bus(&self) -> Rc<EventBus> {
        self.read(|tree| self.node_in(tree).map(|_| tree.bus.clone()))
            .flatten()
            .unwrap_or_else(|| panic!("the bus of {self:?}, which was removed"))
    }

    /// The bus of this scope's tree while the tree is there, whether or not
    /// this scope still is.
    pub(crate) fn tree_bus(&self) -> Option<Rc<EventBus>> {
        self.read(|tree| tree.bus.clone())
    }
}
