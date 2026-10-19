//! The async runtime an application runs on.
//!
//! guinea's actors spawn with bare `tokio::spawn`, which needs a runtime
//! entered on the calling thread - and the thread that installs an application
//! is the UI thread, which nothing has entered. Every application therefore
//! opened with the same two lines before it could do anything:
//!
//! ```
//! let runtime = tokio::runtime::Runtime::new()?;
//! let _guard = runtime.enter();
//! # let spawned = tokio::spawn(async { 2 + 2 });
//! # assert_eq!(runtime.block_on(spawned)?, 4);
//! # anyhow::Ok(())
//! ```
//!
//! So guinea does it instead, unless the application already has a runtime of
//! its own.

use std::sync::OnceLock;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Makes sure this thread can spawn.
///
/// Does nothing when a runtime is already entered here - an application with
/// its own keeps it, and one that starts guinea from inside `#[tokio::main]`
/// does not get a second.
pub(crate) fn ensure_entered() -> anyhow::Result<()> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Ok(());
    }

    let runtime = match RUNTIME.get() {
        Some(runtime) => runtime,
        None => {
            let built = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            RUNTIME.get_or_init(|| built)
        }
    };

    // Leaked on purpose, and safe to leak: the guard borrows a runtime that
    // lives in a `static`, and being entered is a property of the thread for
    // as long as the process runs. Dropping it would take spawning away from
    // the very thread the application lives on.
    std::mem::forget(runtime.enter());
    Ok(())
}
