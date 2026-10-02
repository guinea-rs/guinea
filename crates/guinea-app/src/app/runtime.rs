use std::cell::RefCell;

use guinea_core::SharedState;

use super::builder::FeatureBuilder;

/// An installed application: everything the recipe built, plus the hooks that
/// outlive installation. Held on the UI thread until the process exits.
pub struct AppRuntime {
    pub(crate) builder: FeatureBuilder,
}

thread_local! {
    static RUNTIME: RefCell<Option<AppRuntime>> = const { RefCell::new(None) };
}

/// Hands the runtime to the thread that will tear it down. Call once, from
/// the backend adapter, after [`crate::app::App::install`].
pub fn install_runtime(runtime: AppRuntime) {
    RUNTIME.with(|slot| *slot.borrow_mut() = Some(runtime));
}

/// Whether an application is already installed on this UI thread.
///
/// One application per thread, however many windows it opens: a second window
/// renders its own root, and whatever that root does to bootstrap must be a
/// no-op the second time.
pub fn is_installed() -> bool {
    RUNTIME.with(|slot| slot.borrow().is_some())
}

/// The services plugins provided during installation.
///
/// Empty when there is no installed application - a router built directly in a
/// test, say. Callers get "nothing provided that" rather than a panic, which is
/// the same answer they would get from an application that installed no
/// plugins.
pub fn app_services() -> SharedState {
    RUNTIME.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|runtime| runtime.builder.services.clone())
            .unwrap_or_default()
    })
}

/// The plugins the installed application was built with, by id.
pub fn installed_plugins() -> Vec<&'static str> {
    RUNTIME.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|runtime| runtime.builder.plugin_ids())
            .unwrap_or_default()
    })
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
    builder.lifecycle().clone().shutdown()
}
