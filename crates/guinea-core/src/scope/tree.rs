use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{MAIN, Outlet, Scope, ScopeData};

/// Numbers every scope once, from 1: a node that holds no scope is numbered 0.
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

pub(super) struct Node {
    pub(super) serial: u64,
    pub(super) data: Option<Rc<ScopeData>>,
    pub(super) parent: Option<u32>,
    pub(super) outlet: Outlet,
    pub(super) children: Vec<u32>,
}

/// Every scope in one [`ScopeTree`], and which sits under which.
#[derive(Default)]
pub(super) struct Tree {
    pub(super) nodes: Vec<Node>,
    free: Vec<u32>,
}

impl Tree {
    pub(super) fn insert(&mut self, parent: Option<u32>, outlet: Outlet) -> (u32, u64) {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let data = Some(Rc::new(ScopeData::default()));
        let index = match self.free.pop() {
            Some(index) => {
                let node = &mut self.nodes[index as usize];
                node.serial = serial;
                node.data = data;
                node.parent = parent;
                node.outlet = outlet;
                index
            }
            None => {
                self.nodes.push(Node {
                    serial,
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

        (index, serial)
    }

    pub(super) fn node(&self, index: u32, serial: u64) -> Option<&Node> {
        self.nodes
            .get(index as usize)
            .filter(|node| node.serial == serial && node.data.is_some())
    }

    /// Takes the scope at `index` and everything under it out of the tree,
    /// children before their parent and the newest child first, and hands
    /// back what they held in that order.
    pub(super) fn detach(&mut self, index: u32, serial: u64) -> Vec<Rc<ScopeData>> {
        let Some(node) = self.node(index, serial) else {
            return Vec::new();
        };
        if let Some(parent) = node.parent {
            self.nodes[parent as usize].children.retain(|child| *child != index);
        }

        let mut order = Vec::new();
        self.collect(index, &mut order);

        let mut detached = Vec::with_capacity(order.len());
        for index in order {
            let node = &mut self.nodes[index as usize];
            node.serial = 0;
            node.parent = None;
            node.children.clear();
            if let Some(data) = node.data.take() {
                detached.push(data);
            }
            self.free.push(index);
        }
        detached
    }

    pub(super) fn collect(&self, index: u32, order: &mut Vec<u32>) {
        for child in self.nodes[index as usize].children.iter().rev() {
            self.collect(*child, order);
        }
        order.push(index);
    }
}

thread_local! {
    /// Where a `Scope` finds its tree. Weak: each tree is its [`ScopeTree`]'s,
    /// and the thread ending lets go of nothing here but names.
    static TREES: RefCell<Vec<Weak<RefCell<Tree>>>> = const { RefCell::new(Vec::new()) };
}

/// Lists `tree`, in the first slot whose tree is gone, and says which.
fn register(tree: &Rc<RefCell<Tree>>) -> u32 {
    TREES.with(|trees| {
        let mut trees = trees.borrow_mut();
        let named = Rc::downgrade(tree);

        match trees.iter().position(|slot| slot.strong_count() == 0) {
            Some(slot) => {
                trees[slot] = named;
                slot as u32
            }
            None => {
                trees.push(named);
                (trees.len() - 1) as u32
            }
        }
    })
}

pub(super) fn tree(slot: u32) -> Option<Rc<RefCell<Tree>>> {
    TREES
        .try_with(|trees| trees.borrow().get(slot as usize).and_then(Weak::upgrade))
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
        let tree = Rc::new(RefCell::new(Tree::default()));
        let (index, serial) = tree.borrow_mut().insert(None, MAIN);

        Self {
            root: Scope {
                tree: register(&tree),
                index,
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
