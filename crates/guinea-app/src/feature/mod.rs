pub mod context_ext;
mod host;
mod reach;
mod traits;

pub use context_ext::*;
pub use guinea_core::scope::{Reducer, Scope};
pub use host::FeatureHost;
pub use reach::{At, Here, Lists, Provides, Reaches, Segment, There};
pub use traits::*;
