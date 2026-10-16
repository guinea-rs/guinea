use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use guinea_trace::{
    Cause, Point, Sink, Trace, mark, record_into, resume, thread_default, thread_id, within,
};

fn push(reducer: &'static str) -> Cause {
    mark(|| Point::Push { reducer })
}

fn heard(sink: &Sink) -> (Rc<RefCell<Vec<Cause>>>, guinea_trace::Observing) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let noted = seen.clone();
    let observing = sink.observe(move |trace| match trace {
        Trace::Begin(record) | Trace::Mark(record) => noted.borrow_mut().push(record.id),
        Trace::End { .. } => {}
    });
    (seen, observing)
}

fn taken(sink: &Sink) -> Vec<(u32, Cause)> {
    sink.take_elsewhere()
        .records
        .into_iter()
        .filter_map(|(thread, trace)| match trace {
            Trace::Begin(record) | Trace::Mark(record) => Some((thread, record.id)),
            Trace::End { .. } => None,
        })
        .collect()
}

#[test]
fn a_sink_hears_what_is_recorded_into_it_and_nothing_else() {
    let app = Sink::new();
    let other = Sink::new();
    let (seen, _observing) = heard(&app);

    let into_app = {
        let _recording = record_into(&app);
        push("App")
    };
    {
        let _recording = record_into(&other);
        push("Other");
    }
    push("Nowhere");

    assert_eq!(*seen.borrow(), [into_app]);
}

#[test]
fn a_sink_recorded_into_inside_another_gives_it_back_on_the_way_out() {
    let outer = Sink::new();
    let inner = Sink::new();
    let (outer_seen, _outer) = heard(&outer);
    let (inner_seen, _inner) = heard(&inner);

    let _recording = record_into(&outer);
    let before = push("Before");
    let within_inner = {
        let _recording = record_into(&inner);
        push("Inner")
    };
    let after = push("After");

    assert_eq!(
        (outer_seen.borrow().clone(), inner_seen.borrow().clone()),
        (vec![before, after], vec![within_inner])
    );
}

#[test]
fn what_a_thread_records_with_no_sink_current_goes_to_its_default() {
    let app = Sink::new();
    let test = Sink::new();
    let (app_seen, _app) = heard(&app);
    let (test_seen, _test) = heard(&test);

    let _default = thread_default(&app);
    let defaulted = push("Defaulted");
    let into_test = {
        let _recording = record_into(&test);
        push("Test")
    };

    assert_eq!(
        (app_seen.borrow().clone(), test_seen.borrow().clone()),
        (vec![defaulted], vec![into_test])
    );
}

#[test]
fn a_thread_default_ends_with_its_guard() {
    let app = Sink::new();
    let (seen, _observing) = heard(&app);

    drop(thread_default(&app));
    push("After");

    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());
}

#[test]
fn what_another_thread_records_waits_in_its_own_sink() {
    let app = Sink::new();
    let other = Sink::new();
    let (seen_here, _app) = heard(&app);
    let (_other_seen, _other) = heard(&other);

    let rung = Arc::new(AtomicU32::new(0));
    let ringing = rung.clone();
    app.ring_on_elsewhere(Some(Arc::new(move || {
        ringing.fetch_add(1, Ordering::SeqCst);
    })));

    let worker_app = app.clone();
    let (worker, first, second) = std::thread::spawn(move || {
        let _recording = record_into(&worker_app);
        (thread_id(), push("First"), push("Second"))
    })
    .join()
    .unwrap();

    assert_eq!(
        (
            taken(&app),
            taken(&other),
            seen_here.borrow().clone(),
            rung.load(Ordering::SeqCst)
        ),
        (vec![(worker, first), (worker, second)], vec![], vec![], 1)
    );
}

#[test]
fn a_future_records_into_the_sink_it_was_made_in_wherever_it_is_polled() {
    let app = Sink::new();
    let (_seen, _observing) = heard(&app);

    let cause = {
        let _recording = record_into(&app);
        let action = push("Action");
        let _resumed = resume(Some(action));
        let future = within(Some(action), async { push("Polled") });
        let polled = std::thread::spawn(move || {
            let mut future = std::pin::pin!(future);
            let waker = std::task::Waker::noop();
            let mut cx = std::task::Context::from_waker(waker);
            match future.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(cause) => (thread_id(), cause),
                std::task::Poll::Pending => unreachable!("the future does not wait"),
            }
        })
        .join()
        .unwrap();
        polled
    };

    assert_eq!(taken(&app), [cause]);
}

#[test]
fn a_future_given_a_sink_records_into_it_wherever_it_is_polled() {
    let app = Sink::new();
    let (_seen, _observing) = heard(&app);

    let future = app.within(async { push("Polled") });
    let polled = std::thread::spawn(move || {
        let mut future = std::pin::pin!(future);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(cause) => (thread_id(), cause),
            std::task::Poll::Pending => unreachable!("the future does not wait"),
        }
    })
    .join()
    .unwrap();

    assert_eq!(taken(&app), [polled]);
}

#[test]
fn a_sink_whose_observing_thread_ended_is_observed_by_nobody() {
    let app = Sink::new();

    let observed = app.clone();
    std::thread::spawn(move || std::mem::forget(observed.observe(|_| {})))
        .join()
        .unwrap();

    let _recording = record_into(&app);
    push("Unheard");
    assert_eq!(
        (app.is_observed(), taken(&app)),
        (false, vec![]),
        "a record was kept for an observer that is gone"
    );
}

#[test]
fn two_observers_of_a_sink_both_hear_and_one_leaving_leaves_the_other() {
    let app = Sink::new();
    let (first, hearing_first) = heard(&app);
    let (second, hearing_second) = heard(&app);
    let _recording = record_into(&app);

    let both = push("Both");
    drop(hearing_first);
    let only_second = push("Second");
    let still = app.is_observed();
    drop(hearing_second);

    assert_eq!(
        (
            first.borrow().clone(),
            second.borrow().clone(),
            still,
            app.is_observed()
        ),
        (vec![both], vec![both, only_second], true, false)
    );
}

#[test]
fn nothing_is_built_for_a_sink_nobody_observes() {
    let app = Sink::new();
    let watched = Sink::new();
    let (_seen, _observing) = heard(&watched);
    let _recording = record_into(&app);

    let built = std::cell::Cell::new(false);
    mark(|| {
        built.set(true);
        Point::Plugin {
            kind: "unused",
            fields: Vec::new(),
        }
    });

    assert_eq!((built.get(), taken(&app)), (false, vec![]));
}
