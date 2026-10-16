use std::cell::RefCell;
use std::rc::Rc;

use guinea_trace::{Cause, Point, Sink, Trace, enter, mark, record_into};

#[test]
fn a_point_opened_while_nobody_watched_does_not_end_for_whoever_watches_later() {
    let sink = Sink::new();
    let _recording = record_into(&sink);
    let action = enter(|| Point::Action { message: "Save" });

    let seen = Rc::new(RefCell::new(Vec::new()));
    let noted = seen.clone();
    let observing = sink.observe(move |trace| noted.borrow_mut().push(trace.clone()));

    let pushed = mark(|| Point::Push { reducer: "Metrics" });
    let action_id = action.id();
    drop(action);
    drop(observing);

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
