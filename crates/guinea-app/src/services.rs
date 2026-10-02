//! What plugins provided, reached the same way from wherever it can be.

use std::sync::Arc;

/// A context that can hand out what plugins provided at startup: a feature's
/// [`FeatureInitContext`](crate::feature::FeatureInitContext), a
/// [`PluginBuilder`](crate::app::PluginBuilder), a harness segment, a route's
/// enter guard.
///
/// For a plugin's extension trait to reach its service from any of them at
/// once:
///
/// ```ignore
/// impl<C: Services + ?Sized> StoreAccess for C {
///     fn store(&self) -> Option<Arc<Store>> {
///         self.try_require::<Store>()
///     }
/// }
/// ```
pub trait Services {
    /// The service, or `None` when nothing provided it.
    fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>>;

    /// The service, or an error that names it when nothing provided it.
    fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        self.try_require::<T>().ok_or_else(|| {
            anyhow::anyhow!(
                "no plugin provided service `{}` - install the plugin that provides it on \
                 the application",
                std::any::type_name::<T>()
            )
        })
    }
}

impl Services for crate::feature::ScopeContext {
    fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        crate::feature::ScopeContext::try_require(self)
    }

    fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        crate::feature::ScopeContext::require(self)
    }
}

impl Services for crate::feature::FeatureInitContext {
    fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        crate::feature::ScopeContext::try_require(self)
    }

    fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        crate::feature::ScopeContext::require(self)
    }
}

impl Services for crate::app::PluginBuilder {
    fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        crate::app::PluginBuilder::try_require(self)
    }

    fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        crate::app::PluginBuilder::require(self)
    }
}
