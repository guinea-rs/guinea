//! Navigation and mounting, with no opinion about how anything is drawn.
//!
//! A route is an application's own enum, one level per layout, and a
//! [`Navigator`](navigator::Navigator) over it is a resource of the scope that
//! opened it. A layout mounts what goes under it with a [`Pass`](mount::Pass):
//! a child is the same scope for as long as it is mounted with equal params,
//! and goes when a pass leaves it out. What a view is, and what it is handed,
//! is the backend's business.

extern crate self as guinea_router;

pub mod link;
pub mod manifest;
pub mod mount;
pub mod navigator;
pub mod observability;
#[doc(hidden)]
pub mod sample;
pub mod slot;
