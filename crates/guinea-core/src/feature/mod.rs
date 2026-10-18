//! A reducer's traffic, and the claim that wires it.
//!
//! During install a feature claims a reducer with `cx.state::<R>()`.
//! [`Claim::driven_by`] creates the actor that drives it from that same
//! expression and hands it its [`Push`], so the edge is declared in one place
//! and nothing is left to wire. What the feature keeps is a [`Bound`], which
//! both pushes and emits.
//!
//! After install the traffic runs through two handles: an actor writes the
//! state through [`Push`], the UI asks the domain through [`Dispatch`]. Both
//! name their scope and do not keep it, and both drop what is meant for a
//! scope that has gone.
//!
//! What a feature lets the segments below it read is its [`Exported`] list.

pub(crate) mod claim;
mod dispatch;
pub(crate) mod exports;
mod push;
#[cfg(test)]
mod tests;

pub use claim::Bound;
pub use dispatch::Dispatch;
pub use push::Push;

/// Keeps a handle on the thread its scope lives on: neither `Send` nor
/// `Sync`.
type UiThread = std::marker::PhantomData<*const ()>;
