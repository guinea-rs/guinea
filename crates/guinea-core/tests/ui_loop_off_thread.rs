//! A process of its own, because the dispatcher is process-wide.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use guinea_core::ui_thread::UiLoop;

#[test]
fn a_ui_loop_sees_what_it_waits_for_come_true_off_the_ui_thread() {
    let ui = UiLoop::install();
    let done = Arc::new(AtomicBool::new(false));

    let setting = done.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        setting.store(true, Ordering::SeqCst);
    });

    let started = Instant::now();
    let came = ui.run_until(|| done.load(Ordering::SeqCst), Duration::from_secs(5));
    let waited = started.elapsed();

    assert!(came && waited < Duration::from_secs(1), "came: {came}, after {waited:?}");
}
