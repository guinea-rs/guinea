//! Deep links: the addresses a route tree answers to from outside the
//! application.
//!
//! A route's variant declares its address with `#[at("/files/:path")]`, and
//! the derive writes [`Routable::link`](crate::navigator::Routable::link) and
//! [`Routable::parse`](crate::navigator::Routable::parse) for it. What a
//! capture may carry is closed to [`LinkValue`]; every address a tree answers
//! to is listed as a [`DeepLink`]; and the [`manifest`] writes that list down,
//! so a change to it shows up in review before it breaks a link already sent
//! out.

pub mod manifest;
mod value;

pub use value::LinkValue;

/// One address the application answers to.
///
/// The whole external surface of a route tree, in a shape that can be written
/// down and diffed - see [`manifest`]. What bites about a deep link is not
/// that it is malformed but that it *changed*, and a change is only visible
/// against something committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeepLink {
    /// The route enum this link belongs to. A second window is a second tree,
    /// and its addresses ship just the same.
    pub tree: &'static str,
    /// The place, by its variants down the levels: `Processes::List`.
    pub route: String,
    /// The pattern, captures and all: `/:context/processes`.
    pub path: String,
    /// What the captures carry, in the order the path names them.
    pub captures: Vec<Capture>,
}

/// One capture of a [`DeepLink`], and what it carries.
///
/// The type is here because a path can change meaning without changing shape:
/// `/:id/detail` with `id` moved from `String` to `u32` is the same string and
/// a different address, and a link already sent out stops opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    pub name: &'static str,
    /// [`LinkValue::NAME`], not how the declaration spelled it.
    pub ty: &'static str,
}
