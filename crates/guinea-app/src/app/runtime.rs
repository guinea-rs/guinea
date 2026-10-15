use super::builder::FeatureBuilder;
use crate::feature::ScopeContext;

/// An installed application: everything its function built, plus the hooks
/// that outlive installation. Held on the UI thread by whoever runs the
/// application; letting go of it tears the application down - its windows
/// first, then what it set up, the last first.
pub struct AppRuntime {
    pub(crate) builder: FeatureBuilder,
}

impl AppRuntime {
    /// The application's own context: its scope, and what it provided. What
    /// a backend opens its windows under.
    pub fn context(&self) -> ScopeContext {
        ScopeContext::clone(&self.builder)
    }
}
