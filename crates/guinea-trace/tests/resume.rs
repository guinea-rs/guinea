use guinea_trace::{Cause, Point, Sink, Trace, enter, mark, record_into, resume, thread_id};

/// Every record taken from the other threads, as the thread, its kind, its
/// id and its parent: compared whole, so a record the test did not make
/// fails it.
fn order(records: &[(u32, Trace)]) -> Vec<(u32, &'static str, Cause, Option<Cause>)> {
    records
        .iter()
        .map(|(thread, trace)| match trace {
            Trace::Begin(record) => (*thread, "begin", record.id, record.parent),
            Trace::Mark(record) => (*thread, "mark", record.id, record.parent),
            Trace::End { id, .. } => (*thread, "end", *id, None),
        })
        .collect()
}

#[test]
fn a_cause_carried_to_another_thread_is_the_parent_of_what_is_recorded_there() {
    let sink = Sink::new();
    let observing = sink.observe(|_| {});
    let _recording = record_into(&sink);

    let action = enter(|| Point::Action { message: "Kill" });
    let cause = action.id();

    let worker_sink = sink.clone();
    let (worker, bare, resumed) = std::thread::spawn(move || {
        let _recording = record_into(&worker_sink);
        let bare = mark(|| Point::Push { reducer: "Metrics" });
        let resumed = {
            let _resumed = resume(Some(cause));
            mark(|| Point::Push { reducer: "Metrics" })
        };
        (thread_id(), bare, resumed)
    })
    .join()
    .unwrap();

    drop(action);
    let elsewhere = sink.take_elsewhere();
    drop(observing);

    assert_eq!(
        order(&elsewhere.records),
        [
            (worker, "mark", bare, None),
            (worker, "mark", resumed, Some(cause)),
        ],
        "{elsewhere:#?}"
    );
}
