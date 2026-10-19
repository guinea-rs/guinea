//! Names for what tests and tools reach for on screen.
//!
//! A crate of its own, depending on nothing, so that a widget library can take
//! marks without taking guinea.
//!
//! One enum per application, derived: each variant is a name, written as the
//! variant is.
//!
//! ```
//! # mod guinea {
//! #     pub use guinea_macros::Mark;
//! # }
//! # use guinea_mark::Mark;
//! #[derive(guinea::Mark)]
//! enum Marks {
//!     Search,
//!     Kill,
//! }
//! # assert_eq!([Marks::Search.name(), Marks::Kill.name()], ["Search", "Kill"]);
//! ```

/// A name a backend puts on an element, where a test or a tool finds it again
/// - the `AutomationId` on WinUI.
pub trait Mark: 'static {
    fn name(&self) -> &'static str;
}
