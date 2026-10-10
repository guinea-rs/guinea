pub mod actor;
pub mod binding;
pub mod feature;
pub mod executor;
pub mod guard;
pub mod load;
pub use guinea_mark as mark;
pub mod remote;
pub mod notify;
pub mod observability;
pub mod shared_state;
pub mod scope;
pub mod semantics;
#[cfg(feature = "test-utils")]
pub mod test_kit;
pub mod trace;

pub use load::Load;
pub use shared_state::SharedState;
pub use scope::{Reducer, Scope, StateHandle, Subscription};
pub use actor::{
    Addr, Cx, Handler, ManagedActor, UiDispatcher, UiTask, invoke_on_ui, set_ui_dispatcher,
};
pub use actor::event_bus::subscribe::BusSubscription;

/// What macro output names, so that an application does not have to depend
/// on it too. Not part of the API.
#[doc(hidden)]
pub mod __private {
    pub use anyhow;
    pub use inventory;
    pub use tokio;
    pub use tracing;
}
