//! Where a node lives, and why it is not in the scope.
//!
//! Everywhere else in guinea a segment's state sits in its `Scope`, behind an
//! `Rc<RefCell<_>>`, and every reader takes a snapshot. That works for four
//! backends because their views own everything they show.
//!
//! iced's do not. `text_editor` holds a `&Content` for the life of the
//! element, and a `Ref` taken out of an `Rc` clone dies at the end of the call
//! that took it - so no amount of lifetime plumbing makes a view built that
//! way outlive its own mounting. The state has to be somewhere that hands out
//! `&'a` for exactly the render, and the only such place is what iced itself
//! owns: the shell it passes to `view`.
//!
//! So the nodes live here, and the two things the scope was doing for them -
//! being replaced when a segment reinstalls, and being kept when a page asked
//! to keep its state - are done here too, by the same rules the router uses
//! for the reducers it still owns.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use guinea_core::guard::Verdict;
use guinea_router::router::{SegmentEntry, placement_hash};

use crate::Iced;

/// Where in the mounted chain a node sits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Placement {
    pub cursor: usize,
    pub segment: TypeId,
}

/// A node, what it was mounted with, and its standing answer to "may I be
/// left".
///
/// The verdict is kept beside the node rather than computed on demand because
/// the router asks the *scope*, and a guard registered there cannot reach a
/// store the shell owns. So it is recomputed whenever the node changes, and
/// the registered guard just reads it.
pub(crate) struct Held {
    pub node: Box<dyn Any>,
    pub params: Box<dyn Any>,
    pub verdict: Rc<RefCell<Verdict>>,
}

/// How many departed nodes are kept for pages that asked to keep their state.
const MAX_KEPT: usize = 10;

/// The nodes of the mounted chain.
#[derive(Default)]
pub struct Nodes {
    live: HashMap<Placement, Held>,
    /// What pages with `CACHE_STATE_IN_MEMORY` left behind, keyed the way the
    /// router keys its own cache: by where the segment sat, not by what it is.
    /// The same page under two different layouts is two places.
    kept: HashMap<(u64, usize), Held>,
    order: VecDeque<(u64, usize)>,
    /// The chain these were mounted for, needed to key a departing node before
    /// the new chain replaces it.
    chain: Option<&'static [SegmentEntry<Iced>]>,
}

impl Nodes {
    pub(crate) fn get<P: 'static>(&self, cursor: usize) -> Option<&P> {
        let placement = Placement {
            cursor,
            segment: TypeId::of::<P>(),
        };
        self.live.get(&placement)?.node.downcast_ref()
    }

    /// The node and its verdict slot, for an update that is about to change
    /// it.
    pub(crate) fn get_mut<P: 'static>(
        &mut self,
        cursor: usize,
    ) -> Option<(&mut P, &Rc<RefCell<Verdict>>)> {
        let placement = Placement {
            cursor,
            segment: TypeId::of::<P>(),
        };
        let held = self.live.get_mut(&placement)?;
        let node = held.node.downcast_mut()?;
        Some((node, &held.verdict))
    }

    /// Brings the store in line with the chain the router just installed.
    ///
    /// Called after every navigation. What `install` staged is fresh; what is
    /// still in the chain and was not staged was never reinstalled and keeps
    /// what it had; what left is dropped, or kept if its page asked.
    pub(crate) fn sync(&mut self, chain: &'static [SegmentEntry<Iced>]) {
        let fresh = take_staged();
        let unchanged = self
            .chain
            .is_some_and(|current| std::ptr::eq(current, chain));
        if fresh.is_empty() && unchanged {
            return;
        }

        let leaving = self.chain.take();
        let staged: Vec<Placement> = fresh.iter().map(|(placement, _, _)| *placement).collect();

        for (placement, held) in std::mem::take(&mut self.live) {
            let survives = !staged.contains(&placement)
                && chain
                    .get(placement.cursor)
                    .is_some_and(|entry| (entry.type_id)() == placement.segment);

            if survives {
                self.live.insert(placement, held);
                continue;
            }

            if let Some(previous) = leaving
                && previous
                    .get(placement.cursor)
                    .is_some_and(|entry| entry.cache_state)
            {
                self.keep(
                    (placement_hash(previous, placement.cursor), placement.cursor),
                    held,
                );
            }
        }

        for (placement, held, cache_state) in fresh {
            let held = match cache_state {
                true => self.restored(chain, placement, held),
                false => held,
            };
            self.live.insert(placement, held);
        }

        self.chain = Some(chain);
    }

    /// A kept node, if there is one for this place and it was left under the
    /// same parameters. Otherwise the freshly built one.
    ///
    /// A kept node comes back with its verdict in the fresh one's slot: the
    /// guard the new scope asks reads that slot, not the one the node left
    /// with.
    fn restored(
        &mut self,
        chain: &'static [SegmentEntry<Iced>],
        placement: Placement,
        fresh: Held,
    ) -> Held {
        let key = (placement_hash(chain, placement.cursor), placement.cursor);
        let Some(kept) = self.kept.remove(&key) else {
            return fresh;
        };
        self.order.retain(|held| *held != key);

        let same = (chain[placement.cursor].same_params)(&*kept.params, &*fresh.params);
        // A page kept under one set of parameters is a different page's worth
        // of state under another - the same rule the router applies to the
        // reducers it owns.
        if !same {
            return fresh;
        }

        fresh.verdict.swap(&kept.verdict);
        Held {
            node: kept.node,
            params: kept.params,
            verdict: fresh.verdict,
        }
    }

    fn keep(&mut self, key: (u64, usize), held: Held) {
        if self.kept.insert(key, held).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_KEPT {
            if let Some(oldest) = self.order.pop_front() {
                self.kept.remove(&oldest);
            }
        }
    }
}

thread_local! {
    /// What `install` built, waiting for the shell to take it.
    ///
    /// A staging area because installing happens inside `Router::navigate`,
    /// which has no way to reach the shell's store - and building the node is
    /// the one thing that needs the install context.
    static STAGED: RefCell<Vec<(Placement, Held, bool)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn stage(placement: Placement, held: Held, cache_state: bool) {
    STAGED.with(|staged| staged.borrow_mut().push((placement, held, cache_state)));
}

fn take_staged() -> Vec<(Placement, Held, bool)> {
    STAGED.with(|staged| std::mem::take(&mut *staged.borrow_mut()))
}
