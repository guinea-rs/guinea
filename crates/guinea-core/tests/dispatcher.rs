//! The application's own path to the UI thread: a registered `UiDispatcher`.
//!
//! A process of its own, because the dispatcher is process-wide. It has to
//! hold with `test-utils` on as well as off: `cargo build --all-targets`
//! unifies dev-dependencies' features into the application, and the feature
//! must add a place for work to go, not take the dispatcher away.

use guinea_core::actor::{UiDispatcher, UiTask, invoke_on_ui, set_ui_dispatcher, try_invoke_on_ui};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Recording {
    tasks: Arc<Mutex<Vec<UiTask>>>,
}

impl UiDispatcher for Recording {
    fn init(&self) {}

    fn dispatch(&self, task: UiTask) {
        self.tasks.lock().unwrap().push(task);
    }
}

#[test]
fn work_for_the_ui_thread_goes_to_the_registered_dispatcher() {
    let tasks = Arc::new(Mutex::new(Vec::new()));
    set_ui_dispatcher(Recording {
        tasks: tasks.clone(),
    });

    let ran = Arc::new(AtomicUsize::new(0));

    let counted = ran.clone();
    invoke_on_ui(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });

    let counted = ran.clone();
    assert!(
        try_invoke_on_ui(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .is_ok(),
        "there is a UI thread to run it on"
    );

    let dispatched: Vec<UiTask> = std::mem::take(&mut *tasks.lock().unwrap());
    assert_eq!(dispatched.len(), 2, "both went to the dispatcher");

    for task in dispatched {
        task();
    }
    assert_eq!(ran.load(Ordering::SeqCst), 2);
}
