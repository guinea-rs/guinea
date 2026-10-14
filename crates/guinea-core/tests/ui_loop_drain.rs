//! A frame runs the work that was waiting when it began, and leaves what that
//! work hands over to the next one, so a busy actor cannot starve the screen.
//! Alone in its binary: the UI thread is the process's.

use guinea_core::ui_thread::{UiLoop, invoke_on_ui};

#[test]
fn draining_runs_what_was_waiting_and_leaves_what_it_handed_over() {
    let ui = UiLoop::install();
    invoke_on_ui(|| invoke_on_ui(|| {}));

    let frames = (ui.drain(), ui.drain(), ui.drain());

    assert_eq!(frames, (1, 1, 0));
}
