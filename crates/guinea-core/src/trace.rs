pub use guinea_trace::*;

use crate::scope::Scope;

/// Where the records of `scope`'s tree go, while the tree is there: to
/// [`observe`](Sink::observe) it, or to [`record_into`] it from code that runs
/// for the tree but outside its actors.
pub fn sink_of(scope: Scope) -> Option<Sink> {
    scope.tree_sink()
}
