use once_cell::sync::Lazy;
use std::sync::RwLock;

pub mod addr;
pub mod cancel;
pub mod ctx;
pub mod envelope;
pub mod flow;

pub mod traits;

pub use addr::*;
pub use cancel::*;
pub use ctx::*;
pub use envelope::*;
pub use traits::*;

/// What [`Cx::spawn_source`] takes: the `Stream` of `futures` and
/// `tokio-stream`, named here so a source can be written without either.
pub use futures_core::Stream;

pub mod event_bus;

pub mod registry;
pub mod shape;

pub type UiTask = Box<dyn FnOnce() + Send>;

pub trait UiDispatcher: Send + Sync {
    fn init(&self);
    fn dispatch(&self, task: UiTask);
}

static UI_DISPATCHER: Lazy<RwLock<Option<Box<dyn UiDispatcher>>>> = Lazy::new(|| RwLock::new(None));

pub fn set_ui_dispatcher(dispatcher: impl UiDispatcher + 'static) {
    *UI_DISPATCHER.write().unwrap() = Some(Box::new(dispatcher));
}

#[derive(Clone)]
pub struct UiThreadToken(std::marker::PhantomData<*const ()>);

impl UiThreadToken {
    pub fn dangerously_create_token_unchecked() -> Self {
        Self(std::marker::PhantomData)
    }
}

pub fn invoke_on_ui<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    #[cfg(feature = "test-utils")]
    {
        if let Err(f) = crate::executor::queue_ui(Box::new(f)) {
            crate::actor::event_bus::EventBus::queue_test_task(f);
        }
    }

    #[cfg(not(feature = "test-utils"))]
    {
        if let Some(dispatcher) = UI_DISPATCHER.read().unwrap().as_ref() {
            dispatcher.dispatch(Box::new(f));
        } else {
            panic!(
                "UiDispatcher not initialized! Call guinea_core::actor::set_ui_dispatcher at startup."
            );
        }
    }
}

/// Like [`invoke_on_ui`], but hands `f` back when there is no UI thread to
/// run it on yet.
pub fn try_invoke_on_ui<F>(f: F) -> Result<(), F>
where
    F: FnOnce() + Send + 'static,
{
    #[cfg(feature = "test-utils")]
    {
        if let Err(f) = crate::executor::queue_ui(Box::new(f)) {
            crate::actor::event_bus::EventBus::queue_test_task(f);
        }
        Ok(())
    }

    #[cfg(not(feature = "test-utils"))]
    {
        match UI_DISPATCHER.read().unwrap().as_ref() {
            Some(dispatcher) => {
                dispatcher.dispatch(Box::new(f));
                Ok(())
            }
            None => Err(f),
        }
    }
}

/// The name the trace and the snapshot give `T`: its type and the module it
/// sits in, without the rest of the path - and with its generic arguments,
/// written out in full, so `GenericAgentActor` for one agent is not taken for
/// the same actor for another: `agent::GenericAgentActor<uniproc_agent::windows::WindowsAgent>`.
pub fn short_type_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    let head = full.split('<').next().unwrap_or(full);

    let mut parts = head.rsplitn(3, "::");
    let start = match (parts.next(), parts.next(), parts.next()) {
        (Some(name), Some(module), Some(_)) => head.len() - module.len() - name.len() - "::".len(),
        _ => 0,
    };

    &full[start..]
}

#[cfg(test)]
mod tests {
    use super::short_type_name;

    mod agent {
        pub struct Agent<T>(pub T);
        pub struct Plain;
    }

    mod windows {
        pub struct WindowsAgent;
    }

    #[test]
    fn a_name_keeps_its_module_and_drops_the_rest_of_the_path() {
        assert_eq!(short_type_name::<agent::Plain>(), "agent::Plain");
    }

    #[test]
    fn a_generic_name_keeps_its_arguments_in_full() {
        assert_eq!(
            short_type_name::<agent::Agent<windows::WindowsAgent>>(),
            "agent::Agent<guinea_core::actor::tests::windows::WindowsAgent>"
        );
        assert_ne!(
            short_type_name::<agent::Agent<windows::WindowsAgent>>(),
            short_type_name::<agent::Agent<agent::Plain>>(),
            "one generic actor per argument is told apart"
        );
    }

    #[test]
    fn a_name_with_no_module_is_left_as_it_is() {
        assert_eq!(short_type_name::<u32>(), "u32");
        assert_eq!(short_type_name::<Vec<u32>>(), "vec::Vec<u32>", "the path, not the arguments");
    }
}
