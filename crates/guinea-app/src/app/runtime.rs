use guinea_core::scope::Scope;

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
    /// The application, as a backend opens its windows under it.
    pub fn context(&self) -> AppContext {
        AppContext::of(&self.builder)
    }
}

/// The application, as a backend opens its windows under it: what
/// [`FeatureHost::under`](crate::feature::FeatureHost::under) takes. Only
/// whoever runs the application has one; nothing a feature or a plugin is
/// handed turns into it.
#[derive(Clone)]
pub struct AppContext(pub(crate) ScopeContext);

impl AppContext {
    pub(crate) fn of(app: &FeatureBuilder) -> Self {
        Self(ScopeContext::clone(app))
    }

    /// The application's scope, for the adapters.
    #[doc(hidden)]
    pub fn __scope(&self) -> Scope {
        self.0.scope
    }
}

/// The same application.
impl PartialEq for AppContext {
    fn eq(&self, other: &Self) -> bool {
        self.0.scope == other.0.scope
    }
}
