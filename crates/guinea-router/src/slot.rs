//! A place in a layout's view that something other than the layout fills.

/// A slot: a marker type a layout places, and a segment below it fills.
///
/// Written with `#[slot]` on a unit struct. Placing it in a view is what
/// declares it; there is no list on the layout.
pub trait Slot: 'static {}
