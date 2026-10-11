//! A process of its own, because the dispatcher is process-wide.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use guinea_core::ui_thread::{UiLoop, invoke_on_ui};

#[test]
fn a_ui_loop_runs_work_as_it_comes_until_what_it_waits_for_holds() {
    let ui = UiLoop::install();
    let done = Arc::new(AtomicBool::new(false));

    let setting = done.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        invoke_on_ui(move || setting.store(true, Ordering::SeqCst));
    });

    let came = ui.run_until(|| done.load(Ordering::SeqCst), Duration::from_secs(5));
    let never = ui.run_until(|| false, Duration::from_millis(20));

    assert_eq!((came, never), (true, false));
}
