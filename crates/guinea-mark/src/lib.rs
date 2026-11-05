//! Names for what tests and tools reach for on screen.
//!
//! A crate of its own, depending on nothing, so that a widget library can take
//! marks without taking guinea.
//!
//! Derive an enum per page or widget. Its module, enum and variant form a
//! qualified name, so equally named variants on different screens stay distinct.
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
//! # assert!(Marks::Search.name().ends_with("::Marks::Search"));
//! # assert_ne!(Marks::Search.name(), Marks::Kill.name());
//! ```

/// A name a backend puts on an element, where a test or a tool finds it again
/// - the `AutomationId` on WinUI. Derived names include module, enum and variant.
/// Manual implementations own their namespacing contract.
pub trait Mark: 'static {
    fn name(&self) -> &'static str;
}
