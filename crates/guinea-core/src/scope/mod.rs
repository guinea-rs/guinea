use std::rc::Rc;

use crate::actor::event_bus::EventBus;
use crate::observability::observer::{self, Event};

mod actors;
pub(crate) mod features;
mod held;
mod kept;
mod lifetime;
mod services;
mod state;
mod tree;

#[cfg(test)]
mod tests;

pub use features::{Installed, Listener};
pub(crate) use features::Section;
pub use lifetime::Awake;
pub use state::{DescribedState, Reducer, Slot, StateHandle, Subscription};
pub(crate) use state::Watcher;
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

/// A scope as a tool names it: what tells it apart from every other scope of
/// its tree, and nothing that reaches it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScopeId(u64);

impl ScopeId {
    /// The number behind the id, for something outside the process to name it.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl Scope {
    /// How a tool names this scope.
    pub fn id(&self) -> ScopeId {
        use slotmap::Key;
        ScopeId(self.node.data().as_ffi())
    }

    /// The scope `id` names in this scope's tree, while it is there.
    #[doc(hidden)]
    pub fn named_by(&self, id: ScopeId) -> Option<Scope> {
        let named = self.named(slotmap::KeyData::from_ffi(id.0).into());
        named.is_alive().then_some(named)
    }

    /// The scope at the top of this one's tree, while the tree is there.
    pub(crate) fn root(&self) -> Option<Scope> {
        self.read(|tree| self.named(tree.root))
    }

    /// This scope and every scope under it, each before the ones under it and
    /// the oldest child first, with the scope it sits under.
    pub(crate) fn family(&self) -> Vec<(Scope, Option<Scope>)> {
        self.read(|tree| {
            let mut order = Vec::new();
            if self.node_in(tree).is_some() {
                tree.collect_down(self.node, &mut order);
            }
            order
                .into_iter()
                .map(|(node, parent)| (self.named(node), parent.map(|parent| self.named(parent))))
                .collect()
        })
        .unwrap_or_default()
    }

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
        drop(tree);

        let child = Scope {
            tree: self.tree,
            node,
        };
        observer::tell(child, || Event::ScopeOpened {
            scope: child.id(),
            parent: Some(self.id()),
        });
        child
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
        for (node, data) in detached {
            let gone = self.named(node);
            data.held.let_go(gone);
            observer::tell(gone, || Event::ScopeClosed { scope: gone.id() });
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

    /// The trace sink of this scope's tree while the tree is there.
    pub(crate) fn tree_sink(&self) -> Option<crate::trace::Sink> {
        self.read(|tree| tree.trace.clone())
    }

    /// What redraws this scope's tree, while the tree is there.
    pub(crate) fn tree_drawing(&self) -> Option<Rc<dyn crate::draws::Drawing>> {
        self.read(|tree| tree.drawing.clone()).flatten()
    }

    pub(crate) fn set_tree_drawing(&self, drawing: Rc<dyn crate::draws::Drawing>) {
        let tree = tree(self.tree).unwrap_or_else(|| panic!("drawing {self:?}, whose tree is gone"));
        let replaced = tree.borrow_mut().drawing.replace(drawing);
        drop(tree);
        drop(replaced);
    }

    /// Whether `other` is in this scope's tree.
    pub(crate) fn shares_tree(&self, other: Scope) -> bool {
        self.tree == other.tree
    }
}
