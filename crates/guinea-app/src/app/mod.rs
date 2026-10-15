mod builder;
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
pub use meta::AppMeta;
pub use plugin::{AppFeature, Plugin, Stop};

#[cfg(any(test, feature = "test-utils"))]
pub use acts::{Chain, Shape, Step};
#[cfg(any(test, feature = "test-utils"))]
pub use harness::{Act, Harness, Segment, TestApp, check, check_exclusive};

pub use runtime::AppRuntime;

type Provision = Box<dyn FnOnce(&mut FeatureBuilder) + Send>;
type Registration = Box<dyn FnOnce(&mut FeatureBuilder) -> anyhow::Result<()> + Send>;

/// The application, described before there is a UI thread to build it on: a
/// recipe the backend adapter's `run` replays once, on the UI thread, in the
/// order it was written.
///
/// ```ignore
/// let mut app = GuineaApp::new();
/// app.meta(app_meta!())
///     .plugin(DevToolsPlugin::new())
///     .install::<Activity>();
///
/// run(app, |_| Window::of::<Shell>().title("Processes"))
/// ```
///
/// Named `GuineaApp` rather than `App` because every backend has an `App` of
/// its own and the two appear in the same `main`.
#[derive(Default)]
pub struct GuineaApp {
    meta: Option<AppMeta>,
    services: Vec<Provision>,
    registrations: Vec<Registration>,
}

impl GuineaApp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares who this application is. Provided as a service, so any plugin
    /// can take an `AppMeta` instead of being handed the same strings by hand.
    ///
    /// Installed before anything else, so where it sits in the recipe does
    /// not matter - an application's identity is not a step in a sequence,
    /// and a plugin declared above this call still sees it.
    pub fn meta(&mut self, meta: AppMeta) -> &mut Self {
        self.meta = Some(meta);
        self
    }

    /// Provides a service the application did not build itself.
    ///
    /// For a backend adapter with something only it can offer - the windows
    /// its roots live in, say. Installed before any plugin, like
    /// [`meta`](Self::meta) and for the same reason: a plugin requiring it
    /// must not depend on where this call sat in the recipe.
    pub fn provide<T: Clone + Send + 'static>(&mut self, value: T) -> &mut Self {
        self.services.push(Box::new(move |app| {
            app.provide(value);
        }));
        self
    }

    pub fn plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        self.registrations.push(Box::new(move |app| app.plugin(plugin)));
        self
    }

    /// Installs the application's own feature `F` - see
    /// [`FeatureBuilder::install`].
    pub fn install<F: AppFeature>(&mut self) -> &mut Self {
        self.registrations
            .push(Box::new(|app| app.install::<F>().map(drop)));
        self
    }

    /// Replays the recipe into `builder`: who the application is and the
    /// services first, then every plugin and feature in the order written.
    pub(crate) fn replay(self, builder: &mut FeatureBuilder) -> anyhow::Result<()> {
        if let Some(meta) = self.meta {
            builder.provide(meta);
        }
        for service in self.services {
            service(builder);
        }

        for register in self.registrations {
            register(builder)?;
        }
        Ok(())
    }

    /// Replays the recipe, for a backend adapter's `run`, which holds the
    /// runtime for as long as the application runs and lets go of it after
    /// its windows.
    ///
    /// When a plugin or feature fails, or returns [`Stop`], what was installed
    /// before it is torn down here and the error is returned; an adapter
    /// returns `Ok` from `run` for [`Stop`].
    ///
    /// Fails, before installing anything, off the UI thread: the application
    /// would live where the dispatcher never runs its work, and every answer
    /// from the background would be lost.
    #[doc(hidden)]
    pub fn start(self) -> anyhow::Result<AppRuntime> {
        if guinea_core::ui_thread::on_ui_thread() == Some(false) {
            anyhow::bail!(
                "installed off the UI thread: the dispatcher runs work on another thread, \
                 where this application's actors would not be"
            );
        }

        #[cfg(feature = "own-runtime")]
        runtime_host::ensure_entered()?;

        let mut builder = FeatureBuilder::new();
        self.replay(&mut builder)?;
        Ok(AppRuntime { builder })
    }
}

#[cfg(test)]
mod tests;
