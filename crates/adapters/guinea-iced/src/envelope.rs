//! The one message type iced sees.
//!
//! iced wants a single `Message` at the top of the application; guinea has a
//! tree of nodes that each keep their own. The usual Elm bridge between those
//! two facts is for every parent to grow a variant per child and a `map` per
//! placement - the cost that makes an Elm tree expensive to rearrange.
//!
//! An envelope avoids it by carrying its own delivery: the message, the
//! function that knows how to apply it (monomorphised for the node that made
//! it, so the node type never has to be named anywhere else), and the scope
//! the node was mounted into. Nothing between the widget and the node reads
//! any of it.

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;

use guinea_core::guard::Verdict;
use guinea_core::scope::Scope;
use guinea_router::navigator::Asking;

use crate::{Nodes, UpdateCx};

/// How an envelope gets back to the node that made it.
pub(crate) type Deliver = fn(Scope, &mut Nodes, Box<dyn Any + Send>);

/// What iced routes. Opaque: only this module ever looks inside.
pub struct Envelope(Payload);

enum Payload {
    /// Actors have queued work for this thread. Not addressed to any node -
    /// it is how a background thread asks for a turn.
    Settled,
    /// The answer to a guard's question. Not a node's message either: the
    /// question belongs to the navigator, and so does the answer.
    Answer(bool),
    Node {
        /// The scope of the node that made it. A node mounted again, under
        /// other params, has a scope of its own.
        scope: Scope,
        deliver: Deliver,
        message: Box<dyn Any + Send>,
    },
}

impl Envelope {
    pub(crate) fn settled() -> Self {
        Envelope(Payload::Settled)
    }

    /// What a guard's dialog sends back.
    pub fn answer(allowed: bool) -> Self {
        Envelope(Payload::Answer(allowed))
    }

    pub(crate) fn node(scope: Scope, deliver: Deliver, message: Box<dyn Any + Send>) -> Self {
        Envelope(Payload::Node {
            scope,
            deliver,
            message,
        })
    }
}

/// Applies one node's message to that node.
///
/// Generic over the node and its message rather than over a trait, so pages
/// and layouts share this body without a shared trait having to exist to
/// relate them.
pub(crate) fn deliver<Node, Message>(
    scope: Scope,
    nodes: &mut Nodes,
    message: Box<dyn Any + Send>,
    update: fn(&mut Node, Message, &mut UpdateCx<'_, Node>),
    leaving: fn(&Node) -> Verdict,
) where
    Node: 'static,
    Message: Send + 'static,
{
    // The node can have gone since the widget produced this: a click and a
    // navigation in the same turn, or an observer that fired while its node
    // was being let go. Its scope went with it, and so did its place in the
    // store.
    let Some((node, verdict)) = nodes.get_mut::<Node>(scope) else {
        tracing::debug!(
            node = std::any::type_name::<Node>(),
            "message arrived after its node was let go; dropped"
        );
        return;
    };

    let Ok(message) = message.downcast::<Message>() else {
        tracing::warn!(
            node = std::any::type_name::<Node>(),
            "message of the wrong type for its node; dropped"
        );
        return;
    };

    let mut cx = UpdateCx {
        scope,
        node: std::marker::PhantomData,
    };
    update(node, *message, &mut cx);

    // The navigator asks the scope, and a guard registered there cannot
    // reach this store - so the answer is recomputed here, where the node
    // just changed, and left where the guard can read it.
    *verdict.borrow_mut() = leaving(node);
}

thread_local! {
    /// Messages an observer produced, waiting for the update they will be
    /// applied in. Filled while actors are drained, emptied immediately after
    /// - never left to sit for a frame.
    static PARKED: RefCell<VecDeque<Envelope>> = const { RefCell::new(VecDeque::new()) };
}

pub(crate) fn park(envelope: Envelope) {
    PARKED.with(|queue| queue.borrow_mut().push_back(envelope));
}

fn take_parked() -> Vec<Envelope> {
    PARKED.with(|queue| queue.borrow_mut().drain(..).collect())
}

/// How many times an update may produce more work for itself before this
/// gives up. A node whose `update` observes its way back into its own
/// observer would otherwise spin here instead of at least drawing a frame.
const SETTLE_ROUNDS: usize = 16;

/// Applies one envelope, then everything the observers made of it.
///
/// First lets go of the nodes whose scopes went since the last frame: the
/// frame is over, so nothing borrows them.
pub(crate) fn settle(nodes: &mut Nodes, asking: Option<&dyn Asking>, envelope: Envelope) {
    nodes.sweep();
    apply(nodes, asking, envelope);

    for round in 0.. {
        let parked = take_parked();
        if parked.is_empty() {
            break;
        }

        if round == SETTLE_ROUNDS {
            tracing::warn!(
                dropped = parked.len(),
                "observers still producing messages after {SETTLE_ROUNDS} rounds; \
                 something is translating its own output back into its input"
            );
            break;
        }

        for envelope in parked {
            apply(nodes, asking, envelope);
        }
    }
}

fn apply(nodes: &mut Nodes, asking: Option<&dyn Asking>, envelope: Envelope) {
    match envelope.0 {
        Payload::Settled => {}
        Payload::Answer(allowed) => {
            if let Some(asking) = asking {
                asking.answer(allowed);
            }
        }
        Payload::Node {
            scope,
            deliver,
            message,
        } => deliver(scope, nodes, message),
    }
}
