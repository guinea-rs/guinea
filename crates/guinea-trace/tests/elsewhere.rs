use guinea_trace::{ELSEWHERE_LIMIT, Point, Sink, Trace, mark, record_into, thread_id};

fn note(trace: &Trace) -> Option<&str> {
    match trace {
        Trace::Mark(record) => match &record.point {
            Point::Plugin { kind: "worker", fields } => fields.first().map(|(_, n)| n.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn on_a_thread(sink: &Sink, marks: usize) -> u32 {
    let sink = sink.clone();
    std::thread::spawn(move || {
        let _recording = record_into(&sink);
        for n in 0..marks {
            mark(|| Point::Plugin {
                kind: "worker",
                fields: vec![("n", n.to_string())],
            });
        }
        thread_id()
    })
    .join()
    .unwrap()
}

#[test]
fn what_other_threads_record_waits_for_the_observing_thread_and_keeps_the_newest() {
    let sink = Sink::new();
    let seen_here = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = seen_here.clone();
    let observing = sink.observe(move |_| counted.set(counted.get() + 1));

    let worker = on_a_thread(&sink, 3);
    assert_ne!(worker, thread_id(), "two threads are told apart");

    let taken = sink.take_elsewhere();
    let from_worker: Vec<&str> = taken
        .records
        .iter()
        .filter(|(thread, _)| *thread == worker)
        .filter_map(|(_, trace)| note(trace))
        .collect();
    assert_eq!(from_worker, ["0", "1", "2"]);
    assert_eq!(seen_here.get(), 0, "the observer here hears only this thread");
    assert!(
        sink.take_elsewhere().records.iter().all(|(thread, _)| *thread != worker),
        "a take empties what it took"
    );

    let flood = on_a_thread(&sink, ELSEWHERE_LIMIT + 5);
    let taken = sink.take_elsewhere();
    assert_eq!(taken.records.len(), ELSEWHERE_LIMIT);
    assert!(taken.dropped >= 5, "dropped {}", taken.dropped);
    let last = taken.records.iter().rev().find(|(thread, _)| *thread == flood);
    assert_eq!(
        last.and_then(|(_, trace)| note(trace)),
        Some((ELSEWHERE_LIMIT + 4).to_string().as_str()),
        "the newest stay"
    );

    drop(observing);
    on_a_thread(&sink, 2);
    assert!(sink.take_elsewhere().records.is_empty(), "nobody observes, nothing is kept");
}
