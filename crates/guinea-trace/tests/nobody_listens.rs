//! Alone in its binary: a thread observing anywhere in the process makes
//! every thread record.

use std::cell::Cell;

use guinea_trace::{Point, mark, take_elsewhere};

#[test]
fn nothing_is_built_while_nobody_listens() {
    let built = Cell::new(false);
    mark(|| {
        built.set(true);
        Point::Note("unused".into())
    });

    assert!(!built.get());
    assert!(take_elsewhere().records.is_empty(), "nor kept for anybody");
}
