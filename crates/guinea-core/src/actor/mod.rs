pub(crate) mod addr;
pub(crate) mod cancel;
pub(crate) mod ctx;
mod envelope;
pub(crate) mod flow;
pub(crate) mod traits;

pub use addr::Addr;
pub use cancel::{Cancel, Reason};
pub use ctx::{AsyncContext, Cx, Task, task};
pub use traits::{Handler, ManagedActor, Receives};

/// What [`Task::pour`] and a feature's sources take: the `Stream` of
/// `futures` and `tokio-stream`, named here so a source can be written
/// without either.
pub use futures_core::Stream;

pub mod event_bus;

pub mod snapshot;
pub mod shape;
