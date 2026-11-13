//! Returning background work to WinUI's dispatcher queue.
use guinea_core::ui_thread::{UiDispatcher, UiTask, set_ui_dispatcher};
use std::sync::Mutex;

#[allow(dead_code, non_snake_case, non_camel_case_types, unused_unsafe)]
mod bindings;

struct Proxy(bindings::DispatcherQueue);

impl UiDispatcher for Proxy {
    fn dispatch(&self, task: UiTask) {
        let task = Mutex::new(Some(task));
        let callback = bindings::DispatcherQueueHandler::new(move || {
            if let Some(task) = task.lock().unwrap().take() { task(); }
        });
        match self.0.TryEnqueue(&callback) {
            Ok(true) => {}
            Ok(false) => tracing::debug!("the UI thread no longer takes work; dropped"),
            Err(error) => tracing::debug!(%error, "dispatching UI work"),
        }
    }
}

pub(crate) fn install() -> windows_core::Result<()> {
    set_ui_dispatcher(Proxy(bindings::DispatcherQueue::GetForCurrentThread()?));
    Ok(())
}
