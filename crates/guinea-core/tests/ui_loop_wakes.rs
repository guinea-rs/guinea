//! A loop that sleeps between frames is woken once for a burst of work, not
//! once for each piece. Alone in its binary: the UI thread is the process's.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use guinea_core::ui_thread::{UiLoop, invoke_on_ui};

#[test]
fn a_burst_of_work_wakes_the_loop_once_and_draining_rearms_it() {
    let woken = Arc::new(AtomicUsize::new(0));
    let ui = {
        let woken = woken.clone();
        UiLoop::install_waking(move || {
            woken.fetch_add(1, Ordering::SeqCst);
        })
    };

    let burst = std::thread::spawn(|| {
        for _ in 0..3 {
            invoke_on_ui(|| {});
        }
    });
    burst.join().expect("the burst was handed over");

    let first = woken.load(Ordering::SeqCst);
    let ran = ui.drain();
    invoke_on_ui(|| {});

    assert_eq!((first, ran, woken.load(Ordering::SeqCst)), (1, 3, 2));
}
