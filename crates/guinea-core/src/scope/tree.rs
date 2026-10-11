use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use slotmap::{SlotMap, new_key_type};

use super::{MAIN, Outlet, Scope, ScopeData};
use crate::actor::event_bus::EventBus;

new_key_type! {
    /// Where a scope sits in its tree. A removed scope's key finds nothing,
    /// even once its slot holds another scope.
    pub(super) struct NodeKey;

    /// Where a tree is listed on its thread. A tree that is gone is found by
    /// nothing, even once its slot lists another tree.
    pub(super) struct TreeKey;
}

/// Numbers every scope once, from 1, across every tree: what
/// [`Scope::key`] says.
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

pub(super) struct Node {
    pub(super) serial: u64,
    pub(super) data: Rc<ScopeData>,
    pub(super) parent: Option<NodeKey>,
    pub(super) outlet: Outlet,
    pub(super) children: Vec<NodeKey>,
}

/// Every scope in one [`ScopeTree`], which sits under which, and the bus they
/// share.
pub(super) struct Tree {
    pub(super) nodes: SlotMap<NodeKey, Node>,
    pub(super) bus: Rc<EventBus>,
}

impl Tree {
    fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            bus: Rc::new(EventBus::new()),
        }
    }

    pub(super) fn insert(&mut self, parent: Option<NodeKey>, outlet: Outlet) -> (NodeKey, u64) {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let node = self.nodes.insert(Node {
            serial,
            data: Rc::new(ScopeData::default()),
            parent,
            outlet,
            children: Vec::new(),
        });

        if let Some(parent) = parent {
            self.nodes[parent].children.push(node);
        }

        (node, serial)
    }

    /// Takes the scope at `node` and everything under it out of the tree,
    /// children before their parent and the newest child first, and hands
    /// back what they held in that order.
    pub(super) fn detach(&mut self, node: NodeKey) -> Vec<Rc<ScopeData>> {
        let Some(parent) = self.nodes.get(node).map(|node| node.parent) else {
            return Vec::new();
        };
        if let Some(parent) = parent {
            self.nodes[parent].children.retain(|child| *child != node);
        }

        let mut order = Vec::new();
        self.collect(node, &mut order);

        order
            .into_iter()
            .filter_map(|node| self.nodes.remove(node))
            .map(|node| node.data)
            .collect()
    }

    pub(super) fn collect(&self, node: NodeKey, order: &mut Vec<NodeKey>) {
        for child in self.nodes[node].children.iter().rev() {
            self.collect(*child, order);
        }
        order.push(node);
    }
}

thread_local! {
    /// Where a `Scope` finds its tree. Weak: each tree is its [`ScopeTree`]'s,
    /// and the thread ending lets go of nothing here but names.
    static TREES: RefCell<SlotMap<TreeKey, Weak<RefCell<Tree>>>> =
        RefCell::new(SlotMap::with_key());
}

fn register(tree: &Rc<RefCell<Tree>>) -> TreeKey {
    TREES.with(|trees| trees.borrow_mut().insert(Rc::downgrade(tree)))
}

fn unregister(key: TreeKey) {
    let _ = TREES.try_with(|trees| trees.borrow_mut().remove(key));
}

pub(super) fn tree(key: TreeKey) -> Option<Rc<RefCell<Tree>>> {
    TREES
        .try_with(|trees| trees.borrow().get(key).and_then(Weak::upgrade))
        .ok()
        .flatten()
}

/// Removes its scope when dropped: for a child that nothing else will remove.
#[must_use = "the scope is removed as soon as this is dropped"]
pub struct ScopeGuard(pub(super) Scope);

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

/// A tree of scopes, and the owner of every scope in it: letting go of it
/// removes its root and everything under it, the last first.
///
/// Whoever hosts something holds one - an application, a window with no
/// application around it, a test.
pub struct ScopeTree {
    _tree: Rc<RefCell<Tree>>,
    root: Scope,
}

impl ScopeTree {
    pub fn new() -> Self {
        let tree = Rc::new(RefCell::new(Tree::new()));
        let (node, serial) = tree.borrow_mut().insert(None, MAIN);

        Self {
            root: Scope {
                tree: register(&tree),
                node,
                serial,
            },
            _tree: tree,
        }
    }

    /// The scope at the top of the tree.
    pub fn scope(&self) -> Scope {
        self.root
    }
}

impl Drop for ScopeTree {
    fn drop(&mut self) {
        self.root.remove();
        unregister(self.root.tree);
    }
}

impl Default for ScopeTree {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for ScopeTree {
    type Target = Scope;

    fn deref(&self) -> &Scope {
        &self.root
    }
}
