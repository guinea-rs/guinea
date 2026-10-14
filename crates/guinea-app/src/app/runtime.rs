use super::builder::FeatureBuilder;
use crate::feature::ScopeContext;

/// An installed application: everything the recipe built, plus the hooks that
/// outlive installation. Held on the UI thread by whoever runs the
/// application; letting go of it tears the application down - its windows
/// first, then what it set up, the last first.
pub struct AppRuntime {
    pub(crate) builder: FeatureBuilder,
}

impl AppRuntime {
    /// The application's own context: its scope, and what its plugins
    /// provided. What a backend hands the closure that picks the first route.
    pub fn context(&self) -> ScopeContext {
        ScopeContext::clone(&self.builder)
    }

    /// What was provided as `T`, for whoever holds the application: the
    /// backend picking the first route, the code around `run`.
    pub fn service<T: Clone + 'static>(&self) -> Option<T> {
        self.builder.scope.service::<T>()
    }
}
