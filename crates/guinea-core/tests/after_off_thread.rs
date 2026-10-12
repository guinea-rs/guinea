//! A process of its own, because the UI thread is process-wide.

use std::panic::catch_unwind;
use std::time::Duration;

use guinea_core::ui_thread::UiLoop;

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    match panic.downcast::<String>() {
        Ok(text) => *text,
        Err(panic) => panic.downcast_ref::<&str>().map(|text| text.to_string()).unwrap_or_default(),
    }
}

#[test]
fn after_called_off_the_ui_thread_says_where_to_call_it_from() {
    let _ui = UiLoop::install();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let handle = runtime.handle().clone();

    let called = std::thread::spawn(move || {
        let _entered = handle.enter();
        catch_unwind(|| guinea_core::timers::after(Duration::from_millis(1), || {})).map_err(said)
    })
    .join()
    .unwrap();

    let said = called.expect_err("after() called off the UI thread went quietly, and its closure never runs");
    assert!(said.contains("UI thread"), "{said}");
}
