use guinea_trace::{ELSEWHERE_LIMIT, Point, Trace, mark, observe, stop_observing, take_elsewhere, thread_id};

fn note(trace: &Trace) -> Option<&str> {
    match trace {
        Trace::Mark(record) => match &record.point {
            Point::Note(text) => Some(text.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

fn on_a_thread(marks: usize) -> u32 {
    std::thread::spawn(move || {
        for n in 0..marks {
            mark(|| Point::Note(format!("worker {n}").into()));
        }
        thread_id()
    })
    .join()
    .unwrap()
}

#[test]
fn what_other_threads_record_waits_for_the_observing_thread_and_keeps_the_newest() {
    let seen_here = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = seen_here.clone();
    observe(move |_| counted.set(counted.get() + 1));

    let worker = on_a_thread(3);
    assert_ne!(worker, thread_id(), "two threads are told apart");

    let taken = take_elsewhere();
    let from_worker: Vec<&str> = taken
        .records
        .iter()
        .filter(|(thread, _)| *thread == worker)
        .filter_map(|(_, trace)| note(trace))
        .collect();
    assert_eq!(from_worker, ["worker 0", "worker 1", "worker 2"]);
    assert_eq!(seen_here.get(), 0, "the observer here hears only this thread");
    assert!(
        take_elsewhere().records.iter().all(|(thread, _)| *thread != worker),
        "a take empties what it took"
    );

    let flood = on_a_thread(ELSEWHERE_LIMIT + 5);
    let taken = take_elsewhere();
    assert_eq!(taken.records.len(), ELSEWHERE_LIMIT);
    assert!(taken.dropped >= 5, "dropped {}", taken.dropped);
    let last = taken.records.iter().rev().find(|(thread, _)| *thread == flood);
    assert_eq!(
        last.and_then(|(_, trace)| note(trace)),
        Some(format!("worker {}", ELSEWHERE_LIMIT + 4).as_str()),
        "the newest stay"
    );

    stop_observing();
    on_a_thread(2);
    assert!(take_elsewhere().records.is_empty(), "nobody observes, nothing is kept");
}
