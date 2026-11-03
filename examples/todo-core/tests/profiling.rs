#![cfg(feature = "profile")]
use guinea::app::Harness;
use todo_core::profiling::{Report, session, verify_saved};

#[test]
fn the_reference_session_edits_searches_undoes_and_survives_a_restart() {
    for seed in 0..4 {
        let storage = tempfile::tempdir().unwrap();
        let path = storage.path().join("todo.json");
        let mut app = Harness::new(seed);
        let report = session(&mut app, &path, 64, |_| Ok(())).unwrap();
        assert_eq!(
            report,
            Report {
                tasks: 64,
                revision: 108,
                matches: 32,
                journal_entries: 20
            }
        );
        drop(app);
        verify_saved(&path, &report).unwrap();
    }
}

#[test]
fn the_reference_trace_comes_from_the_real_application_handlers() {
    use guinea::observability::trace::{self, Point, Trace};
    use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

    #[derive(Debug, Default)]
    struct Surface {
        actions: BTreeSet<&'static str>,
        handlers: BTreeSet<(&'static str, &'static str)>,
        reducers: BTreeSet<&'static str>,
        events: BTreeSet<&'static str>,
        tasks: BTreeSet<&'static str>,
    }
    let storage = tempfile::tempdir().unwrap();
    let mut app = Harness::new(42);
    let surface = Rc::new(RefCell::new(Surface::default()));
    let observed = surface.clone();
    let sink = trace::sink_of(app.application_scope()).unwrap();
    let subscription = sink.observe(move |event| {
        let record = match event {
            Trace::Begin(record) | Trace::Mark(record) => record,
            Trace::End { .. } => return,
        };
        let mut surface = observed.borrow_mut();
        match record.point {
            Point::Action { message } => {
                surface.actions.insert(message);
            }
            Point::Handle { actor, message } => {
                surface.handlers.insert((actor, message));
            }
            Point::Push { reducer } => {
                surface.reducers.insert(reducer);
            }
            Point::Publish { event, .. } => {
                surface.events.insert(event);
            }
            Point::Spawn { output, .. } => {
                surface.tasks.insert(output);
            }
            _ => {}
        }
    });
    session(&mut app, &storage.path().join("todo.json"), 64, |_| Ok(())).unwrap();
    drop(app);
    drop(subscription);
    insta::assert_debug_snapshot!(&*surface.borrow());
}
