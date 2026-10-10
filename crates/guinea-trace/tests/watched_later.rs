//! Alone in its binary: a thread observing anywhere in the process makes
//! every thread record, and this needs nobody to be watching at first.

use std::cell::RefCell;
use std::rc::Rc;

use guinea_trace::{Cause, Point, Trace, enter, mark, observe, stop_observing};

#[test]
fn a_point_opened_while_nobody_watched_does_not_end_for_whoever_watches_later() {
    let action = enter(|| Point::Action { message: "Save" });

    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    observe(move |trace| sink.borrow_mut().push(trace.clone()));

    let pushed = mark(|| Point::Push { reducer: "Metrics" });
    let action_id = action.id();
    drop(action);
    stop_observing();

    let seen: Vec<(&str, Cause, Option<Cause>)> = seen
        .borrow()
        .iter()
        .map(|trace| match trace {
            Trace::Begin(record) => ("begin", record.id, record.parent),
            Trace::Mark(record) => ("mark", record.id, record.parent),
            Trace::End { id, .. } => ("end", *id, None),
        })
        .collect();
    assert_eq!(seen, [("mark", pushed, Some(action_id))]);
}
