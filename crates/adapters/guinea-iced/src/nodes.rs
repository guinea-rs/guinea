//! Where a node lives, and why it is not in the scope.
//!
//! On every other backend a node's state sits in its `Scope`, behind an
//! `Rc<RefCell<_>>`, and every reader takes a snapshot. That works there
//! because those views own everything they show.
//!
//! iced's do not. `text_editor` holds a `&Content` for the life of the
//! element, and a `Ref` taken out of an `Rc` clone dies at the end of the call
//! that took it - so no amount of lifetime plumbing makes a view built that
//! way outlive its own drawing. The state has to be somewhere that hands out
//! `&'a` for exactly the frame, and the only such place is what iced itself
//! owns: the shell it passes to `view`.
//!
//! A layout mounts while it draws, so the store also takes a node in through
//! `&self`, in the middle of a frame that already lent out others. It is
//! append-only for that reason: a node is boxed and never moved while a frame
//! can see it, and the nodes of scopes that went are dropped in `update`,
//! where nothing is lent.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use elsa::FrozenMap;
use guinea_core::guard::Verdict;
use guinea_core::scope::Scope;

/// A node, and its standing answer to "may I be left".
///
/// The verdict is kept beside the node rather than computed on demand because
/// the navigator asks the *scope*, and a guard registered there cannot reach a
/// store the shell owns. So it is recomputed whenever the node changes, and
/// the registered guard just reads it.
pub(crate) struct Held {
    pub node: Box<dyn Any>,
    pub verdict: Rc<RefCell<Verdict>>,
}

/// What `install` built, left in the scope until the store adopts it.
///
/// `install` runs inside a pass, which has no way to reach the shell's store.
struct Staged(Option<Held>);

/// The nodes of what is mounted, by the scope each was mounted into.
#[derive(Default)]
pub struct Nodes {
    held: FrozenMap<Scope, Box<Held>>,
}

impl Nodes {
    pub(crate) fn get<N: 'static>(&self, scope: Scope) -> Option<&N> {
        self.held.get(&scope)?.node.downcast_ref()
    }

    /// The node and its verdict slot, for an update that is about to change
    /// it.
    pub(crate) fn get_mut<N: 'static>(&mut self, scope: Scope) -> Option<(&mut N, &Rc<RefCell<Verdict>>)> {
        let held = self.held.as_mut().get_mut(&scope)?;
        let node = held.node.downcast_mut()?;
        Some((node, &held.verdict))
    }

    /// Takes in the node `install` left in `scope`, when there is one: a node
    /// mounted by the frame that is drawing it.
    pub(crate) fn adopt(&self, scope: Scope) {
        if self.held.get(&scope).is_some() || !scope.keeps::<Staged>() {
            return;
        }
        if let Some(held) = scope.kept::<Staged, _>(|staged| staged.0.take()).flatten() {
            self.held.insert(scope, Box::new(held));
        }
    }

    /// Drops the nodes whose scopes are gone.
    pub(crate) fn sweep(&mut self) {
        self.held.as_mut().retain(|scope, _| scope.is_alive());
    }
}

pub(crate) fn stage(scope: Scope, held: Held) {
    scope.keep(Staged(Some(held)));
}
