//! A process of its own, because the dispatcher is process-wide.

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use guinea_core::ui_thread::{UiLoop, invoke_on_ui, on_ui_thread};

#[test]
fn a_ui_loop_runs_what_is_handed_to_the_ui_thread_on_its_own_thread_and_only_when_asked() {
    let ui = UiLoop::install();
    let ran_on: Arc<Mutex<Vec<ThreadId>>> = Arc::default();

    let noted = ran_on.clone();
    std::thread::spawn(move || {
        invoke_on_ui(move || {
            let again = noted.clone();
            noted.lock().unwrap().push(std::thread::current().id());
            invoke_on_ui(move || again.lock().unwrap().push(std::thread::current().id()));
        });
    })
    .join()
    .unwrap();
    let before = ran_on.lock().unwrap().len();

    let ran = ui.run();

    let here = std::thread::current().id();
    assert_eq!((before, ran, on_ui_thread()), (0, 2, Some(true)));
    assert_eq!(*ran_on.lock().unwrap(), [here, here]);
}
