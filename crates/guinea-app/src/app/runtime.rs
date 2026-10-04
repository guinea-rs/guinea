use std::cell::RefCell;

use super::builder::FeatureBuilder;
use crate::feature::ScopeContext;
use crate::observability::Observed;

/// An installed application: everything the recipe built, plus the hooks that
/// outlive installation. Held on the UI thread until the process exits.
pub struct AppRuntime {
    pub(crate) builder: FeatureBuilder,
    pub(crate) _observed: Observed,
}

impl AppRuntime {
    /// The application's own context: its scope, and what its plugins
    /// provided. What a backend hands the closure that picks the first route.
    pub fn context(&self) -> ScopeContext {
        ScopeContext::clone(&self.builder)
    }
}

thread_local! {
    static RUNTIME: RefCell<Option<AppRuntime>> = const { RefCell::new(None) };
}

/// Hands the runtime to the thread that will tear it down. Call once, from
/// the backend adapter, after [`crate::app::App::install`].
pub fn install_runtime(runtime: AppRuntime) {
    RUNTIME.with(|slot| *slot.borrow_mut() = Some(runtime));
}

/// Runs cleanups and reports actors that outlived them. Called from the
/// reactor's exit callback, on the UI thread.
pub fn shutdown_current() {
    let Some(runtime) = RUNTIME.with(|slot| slot.borrow_mut().take()) else {
        return;
    };

    teardown(&runtime.builder);
}

pub(crate) fn teardown(builder: &FeatureBuilder) -> Vec<(&'static str, usize)> {
    builder.host().shutdown()
}
