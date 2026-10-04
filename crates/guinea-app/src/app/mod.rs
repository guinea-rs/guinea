mod builder;
mod host;
mod meta;
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
pub use host::AppHost;
pub use meta::AppMeta;
pub use plugin::{AppFeature, Application, Installed, Plugin, Stop};

#[cfg(any(test, feature = "test-utils"))]
pub use acts::{Chain, Shape, Step};
#[cfg(any(test, feature = "test-utils"))]
pub use harness::{Act, Harness, Segment, TestApp, check, check_exclusive};

pub use runtime::{AppRuntime, install_runtime, shutdown_current};

use guinea_core::actor::UiThreadToken;

pub type Registration = Box<dyn FnOnce(&mut FeatureBuilder) -> anyhow::Result<()> + Send>;
pub type ReadyHook = Box<dyn FnOnce(&mut FeatureBuilder) + Send>;

/// The application, described before there is a UI thread to build it on.
///
/// Everything registered here is replayed once by the backend adapter, on the
/// UI thread, in registration order. Named `GuineaApp` rather than `App`
/// because every backend has an `App` of its own and the two appear in the
/// same `main`.
#[derive(Default)]
pub struct GuineaApp {
    meta: Option<AppMeta>,
    services: Vec<ReadyHook>,
    registrations: Vec<Registration>,
    ready: Vec<ReadyHook>,
}

impl GuineaApp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares who this application is. Provided as a service, so any plugin
    /// can ask for it with `require::<AppMeta>()` instead of being handed the
    /// same strings by hand.
    ///
    /// Installed before anything else, so where it sits in the chain does not
    /// matter - an application's identity is not a step in a sequence, and a
    /// plugin declared above this call still sees it.
    pub fn meta(mut self, meta: AppMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Provides a service the application did not build itself.
    ///
    /// For a backend adapter with something only it can offer - the windows
    /// its roots live in, say. Installed before any plugin, like
    /// [`meta`](Self::meta) and for the same reason: a plugin requiring it
    /// must not depend on where this call sat in the chain.
    pub fn provide<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.services.push(Box::new(move |app| {
            app.provide(value);
        }));
        self
    }

    pub fn plugin<P: Plugin>(mut self, plugin: P) -> Self {
        self.registrations
            .push(Box::new(move |app| app.plugin(plugin).map(|_| ())));
        self
    }

    pub fn feature<F: AppFeature>(mut self, feature: F) -> Self {
        self.registrations
            .push(Box::new(move |app| app.feature(feature).map(|_| ())));
        self
    }

    /// Installs `A`, the application a route tree names with `app = ..`.
    pub fn application<A: Application>(mut self) -> Self {
        self.registrations.push(Box::new(|app| {
            A::install(app)?;
            app.scope.mark_feature_installed::<A>();
            Ok(())
        }));
        self
    }

    /// Runs once everything is installed, before the first render.
    pub fn on_ready(mut self, f: impl FnOnce(&mut FeatureBuilder) + Send + 'static) -> Self {
        self.ready.push(Box::new(f));
        self
    }

    /// Replays the recipe: installs every plugin and feature in registration
    /// order, then runs the ready hooks.
    ///
    /// For backend adapters. The caller must already be on the UI thread -
    /// that is what `token` attests to - and must hand the result to
    /// [`install_runtime`] so teardown can find it.
    ///
    /// When a plugin or feature fails, or returns [`Stop`], what was installed
    /// before it is torn down here and the error is returned; an adapter
    /// returns `Ok` from `run` for [`Stop`].
    pub fn install(self, token: UiThreadToken) -> anyhow::Result<AppRuntime> {
        // Before anything is built: a feature may spawn an actor while
        // installing, and that needs a runtime on this thread.
        #[cfg(feature = "own-runtime")]
        runtime_host::ensure_entered()?;

        let mut builder = FeatureBuilder::new(token.clone(), AppHost::new());

        // First, whatever the order of the calls that built this: a plugin
        // asking who the application is must not depend on where `meta()` sat
        // in the chain.
        if let Some(meta) = self.meta {
            builder.provide(meta);
        }
        for service in self.services {
            service(&mut builder);
        }

        for register in self.registrations {
            if let Err(error) = register(&mut builder) {
                runtime::teardown(&builder);
                return Err(error);
            }
        }
        for hook in self.ready {
            hook(&mut builder);
        }

        Ok(AppRuntime {
            _observed: builder.observed(),
            builder,
        })
    }
}

#[cfg(test)]
mod tests;
