extern crate self as guinea_core;

pub mod actor;
pub mod binding;
pub mod feature;
pub mod feed;
pub mod executor;
pub mod guard;
pub mod inspect;
pub mod load;
pub use guinea_mark as mark;
mod remote;
pub mod notify;
pub mod observability;
pub mod scope;
pub mod semantics;
pub mod sources;
pub mod trace;
mod type_name;
pub mod ui_thread;

pub use load::Load;
pub use type_name::short_type_name;
pub use scope::Reducer;

/// What macro output names, so that an application does not have to depend
/// on it too, and what only macros implement. Not part of the API.
#[doc(hidden)]
pub mod __private {
    pub use anyhow;
    pub use inventory;
    pub use tokio;

    pub use crate::actor::addr::{Home, HomeGuard};
    pub use crate::actor::flow::{Allows, EventSubscription, Open};
    pub use crate::actor::traits::PublishedBy;
    pub use crate::feature::claim::{Claim, Serves};
    pub use crate::feature::exports::{Exported, Named};
    pub use crate::scope::features::Section;

    pub mod observer {
        pub use crate::observability::observer::{Describer, tell, watch};
    }

    pub mod remote {
        pub use crate::remote::*;
    }
}
