mod builder;
mod meta;
mod observed;
#[cfg(feature = "own-runtime")]
mod runtime_host;
mod plugin;
pub(crate) mod registry;
pub mod roots;
pub(crate) mod runtime;
pub mod windows;

#[cfg(any(test, feature = "test-utils"))]
mod acts;
#[cfg(any(test, feature = "test-utils"))]
mod harness;

pub use builder::{FeatureBuilder, PluginBuilder};
pub use meta::AppMeta;
pub use observed::Observed;
pub use plugin::{AppFeature, Plugin, Stop};

#[cfg(any(test, feature = "test-utils"))]
pub use acts::{Chain, Shape, Step};
#[cfg(any(test, feature = "test-utils"))]
pub use harness::{Act, Harness, Segment, TestApp, check, check_exclusive};

pub use runtime::{AppContext, AppRuntime};

/// Builds the application with `app`, for a backend adapter's `run`, once
/// its UI thread is ready: the application's function installs plugins and
/// features in the order it is written and returns its window, which the
/// adapter opens. `backend` is what devtools call the adapter: `"winui"`,
/// `"egui"`.
///
/// The adapter holds the runtime for as long as the application runs and lets
/// go of it after its windows. When `app` fails, or returns [`Stop`], what it
/// installed is torn down here and the error is returned; an adapter returns
/// `Ok` from `run` for [`Stop`].
///
/// Fails, before installing anything, off the UI thread: the application
/// would live where the dispatcher never runs its work, and every answer from
/// the background would be lost.
#[doc(hidden)]
pub fn start<W>(
    backend: &'static str,
    app: impl FnOnce(&mut FeatureBuilder) -> anyhow::Result<W>,
) -> anyhow::Result<(AppRuntime, W)> {
    if guinea_core::ui_thread::on_ui_thread() == Some(false) {
        anyhow::bail!(
            "installed off the UI thread: the dispatcher runs work on another thread, \
             where this application's actors would not be"
        );
    }

    #[cfg(feature = "own-runtime")]
    runtime_host::ensure_entered()?;

    let mut builder = FeatureBuilder::new();
    builder.__scope().kept::<registry::Backend, _>(|named| named.0 = Some(backend));

    let window = app(&mut builder)?;
    Ok((AppRuntime { builder }, window))
}

#[cfg(test)]
mod tests;
