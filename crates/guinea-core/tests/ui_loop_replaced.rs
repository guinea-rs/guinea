//! A `UiLoop` another one replaced. A process of its own, because the
//! dispatcher is process-wide and replacing it takes work from other tests.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use guinea_core::ui_thread::UiLoop;

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    match panic.downcast::<String>() {
        Ok(text) => *text,
        Err(panic) => panic.downcast_ref::<&str>().map(|text| text.to_string()).unwrap_or_default(),
    }
}

#[test]
fn a_loop_another_one_replaced_says_so_instead_of_waiting_for_nothing() {
    let first = UiLoop::install();
    let _second = UiLoop::install();

    let waited = catch_unwind(AssertUnwindSafe(|| first.run_until(|| false, Duration::from_millis(200))));
    let ran = catch_unwind(AssertUnwindSafe(|| first.run()));

    let waited = said(waited.expect_err("run_until waited out its time"));
    assert!(waited.contains("replaced"), "{waited}");
    let ran = said(ran.expect_err("run returned as if nothing was there"));
    assert!(ran.contains("replaced"), "{ran}");
}
