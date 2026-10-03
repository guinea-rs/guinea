//! Getting work back onto the UI thread, through the application's proxy.

use guinea_core::actor::{UiDispatcher, UiTask, set_ui_dispatcher};
use windows_reactor::AppProxy;

struct Proxy(AppProxy);

impl UiDispatcher for Proxy {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        let run = move |_: &windows_reactor::AppContext| {
            task();
            Ok(())
        };
        if let Err(error) = self.0.dispatch(run) {
            tracing::debug!(%error, "the UI thread no longer takes work; dropped");
        }
    }
}

pub(crate) fn install(proxy: AppProxy) {
    set_ui_dispatcher(Proxy(proxy));
}
