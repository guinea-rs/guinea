//! The one message type iced sees.
//!
//! iced wants a single `Message` at the top of the application; guinea has a
//! chain of nodes that each keep their own. The usual Elm bridge between those
//! two facts is for every parent to grow a variant per child and a `map` per
//! placement - the cost that makes an Elm tree expensive to rearrange.
//!
//! An envelope avoids it by carrying its own delivery: the message, the
//! function that knows how to apply it (monomorphised for the node that made
//! it, so the node type never has to be named anywhere else), and where in the
//! chain that node sat and in which scope. Nothing between the widget and the
//! node reads any of it.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use guinea_core::guard::Verdict;
use guinea_core::scope::Scope;
use guinea_router::router::{Router, SegmentProps};

use crate::{Iced, Nodes, UpdateCx};

/// How an envelope gets back to the node that made it.
pub(crate) type Deliver = fn(&SegmentProps<Iced>, &mut Nodes, Box<dyn Any + Send>);

/// What iced routes. Opaque: only this module ever looks inside.
pub struct Envelope(Payload);

enum Payload {
    /// Actors have queued work for this thread. Not addressed to any node -
    /// it is how a background thread asks for a turn.
    Settled,
    /// The answer to a guard's question. Not a node's message either: the
    /// question belongs to the router, and so does the answer.
    Answer(bool),
    Node {
        cursor: usize,
        /// The scope of the segment that made it. A node installed again at
        /// `cursor`, under other params, has a scope of its own.
        drawn: Scope,
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

    pub(crate) fn new(
        cursor: usize,
        drawn: Scope,
        deliver: Deliver,
        message: Box<dyn Any + Send>,
    ) -> Self {
        Envelope(Payload::Node {
            cursor,
            drawn,
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
    props: &SegmentProps<Iced>,
    nodes: &mut Nodes,
    message: Box<dyn Any + Send>,
    update: fn(&mut Node, Message, &mut UpdateCx<'_, Node>),
    leaving: fn(&Node) -> Verdict,
) where
    Node: 'static,
    Message: Send + 'static,
{
    // The chain can have moved since the widget produced this: a click and a
    // navigation in the same turn, or an observer that fired while the node it
    // belongs to was being torn down. Delivering to whatever now occupies the
    // position would be worse than dropping it.
    if (props.chain[props.cursor].type_id)() != TypeId::of::<Node>() {
        tracing::debug!(
            node = std::any::type_name::<Node>(),
            cursor = props.cursor,
            "message arrived after its node left the chain; dropped"
        );
        return;
    }

    let Ok(message) = message.downcast::<Message>() else {
        tracing::warn!(
            node = std::any::type_name::<Node>(),
            "message of the wrong type for its node; dropped"
        );
        return;
    };

    let Some((node, verdict)) = nodes.get_mut::<Node>(props.cursor) else {
        return;
    };

    let mut cx = UpdateCx {
        props,
        segment: std::marker::PhantomData,
    };
    update(node, *message, &mut cx);

    // The router asks the scope, and a guard registered there cannot reach
    // this store - so the answer is recomputed here, where the node just
    // changed, and left where the guard can read it.
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
pub(crate) fn settle(router: &Rc<Router<Iced>>, nodes: &mut Nodes, envelope: Envelope) {
    apply(router, nodes, envelope);

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
            apply(router, nodes, envelope);
        }
    }

    // Anything above may have navigated - directly, or by answering a guard -
    // and installing stages new nodes rather than storing them, because it
    // runs inside the router with no way to reach this store.
    if let Some(chain) = router.active_chain() {
        nodes.sync(chain);
    }
}

fn apply(router: &Rc<Router<Iced>>, nodes: &mut Nodes, envelope: Envelope) {
    match envelope.0 {
        Payload::Settled => {}
        Payload::Answer(allowed) => router.answer(allowed),
        Payload::Node {
            cursor,
            drawn,
            deliver,
            message,
        } => {
            let Some(props) = props_at(router, cursor) else {
                return;
            };

            if props.scope() != drawn {
                tracing::debug!(
                    cursor,
                    "message arrived after the segment that made it was installed again; dropped"
                );
                return;
            }

            deliver(&props, nodes, message);
        }
    }
}

fn props_at(router: &Rc<Router<Iced>>, cursor: usize) -> Option<SegmentProps<Iced>> {
    let chain = router.active_chain()?;
    let scopes = router.active_scopes()?;
    (cursor < chain.len() && cursor < scopes.len()).then_some(SegmentProps {
        chain,
        scopes,
        cursor,
    })
}
