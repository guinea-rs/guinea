//! Alone in its binary: whether anyone observes is process-wide.

use guinea_trace::{Point, is_observed_anywhere, mark, observe, take_elsewhere};

#[test]
fn a_thread_that_observed_and_ended_is_watching_nothing() {
    std::thread::spawn(|| observe(|_| {})).join().unwrap();

    assert!(
        !is_observed_anywhere(),
        "the observing thread is gone, yet devtools are said to watch"
    );

    mark(|| Point::Plugin {
        kind: "unheard",
        fields: Vec::new(),
    });
    assert!(
        take_elsewhere().records.is_empty(),
        "a record was kept for an observer that is gone"
    );
}
