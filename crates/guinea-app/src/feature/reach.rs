//! Reading from a segment, whatever backend draws it.
//!
//! What a segment may read is answered where it reads: the scope that owns
//! the reducer is found by walking up from the segment, through what each
//! segment above it installed and listed in `Exports`. A read that reaches
//! nothing says which chain it walked, on the render that makes it.

use std::rc::Rc;

use guinea_core::feature::Dispatch;
use guinea_core::scope::Reducer;

/// A page or a layout being drawn, as far as reading goes - whatever backend
/// draws it.
///
/// What lets a plugin offer one shortcut on every backend, the way
/// [`Services`](crate::services::Services) lets it offer one on every context
/// that holds services. A segment reads `R` through it and is drawn again when
/// `R` changes: by subscribing, or by drawing every frame anyway. A backend
/// that builds its view once and binds it is not one of these - it binds.
pub trait Reads {
    /// `R` as it is now, and what may be asked of the feature that owns it.
    fn read<R>(&mut self) -> (Rc<R>, Dispatch)
    where
        R: Reducer + PartialEq;

    /// What may be asked of the feature that owns `R`, without reading it:
    /// the segment is not drawn again when `R` changes.
    fn dispatch<R>(&self) -> Dispatch
    where
        R: Reducer;
}
