//! The UI thread: the one thread actors, scopes and buses live on, and how
//! work from any other thread gets onto it.
//!
//! A toolkit's backend installs a [`UiDispatcher`] that hands work to its own
//! loop; code with no toolkit installs a [`UiLoop`].

use once_cell::sync::Lazy;
use std::sync::RwLock;

mod ui_loop;

pub use ui_loop::UiLoop;

pub type UiTask = Box<dyn FnOnce() + Send>;

pub trait UiDispatcher: Send + Sync {
    fn init(&self);
    fn dispatch(&self, task: UiTask);
}

static UI_DISPATCHER: Lazy<RwLock<Option<Box<dyn UiDispatcher>>>> = Lazy::new(|| RwLock::new(None));

/// Installs the dispatcher [`invoke_on_ui`] hands work to. Called from the
/// UI thread: the thread it is called on is the one that work runs on.
pub fn set_ui_dispatcher(dispatcher: impl UiDispatcher + 'static) {
    *UI_DISPATCHER.write().unwrap() = Some(Box::new(dispatcher));
    *UI_THREAD.write().unwrap() = Some(std::thread::current().id());
}

static UI_THREAD: RwLock<Option<std::thread::ThreadId>> = RwLock::new(None);

/// Whether this is the UI thread, the one [`set_ui_dispatcher`] was called
/// on; `None` while no dispatcher is installed, as in a test.
pub fn on_ui_thread() -> Option<bool> {
    let ui = (*UI_THREAD.read().unwrap())?;
    Some(ui == std::thread::current().id())
}

/// Runs `f` on the UI thread.
///
/// Under `test-utils` a seeded executor on this thread takes it first, then
/// the application's dispatcher.
///
/// # Panics
///
/// When neither is there to take it.
pub fn invoke_on_ui<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if try_invoke_on_ui(f).is_err() {
        panic!("UiDispatcher not initialized! Call guinea_core::ui_thread::set_ui_dispatcher at startup.");
    }
}

/// Like [`invoke_on_ui`], but hands `f` back when there is no UI thread to
/// run it on yet.
pub fn try_invoke_on_ui<F>(f: F) -> Result<(), F>
where
    F: FnOnce() + Send + 'static,
{
    #[cfg(feature = "test-utils")]
    let f = match crate::executor::queue_ui(f) {
        Ok(()) => return Ok(()),
        Err(f) => f,
    };

    match UI_DISPATCHER.read().unwrap().as_ref() {
        Some(dispatcher) => {
            dispatcher.dispatch(Box::new(f));
            Ok(())
        }
        None => Err(f),
    }
}
