use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use guinea::app::Harness;
use todo_core::*;

fn start(h: &mut Harness) -> Rc<RefCell<Vec<Snapshot>>> {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let saved = writes.clone();
    h.app(move |app| {
        configure(
            app,
            Repository::new(Snapshot::default(), move |snapshot| {
                saved.borrow_mut().push(snapshot);
                Ok(())
            }),
        )
    })
    .unwrap();
    writes
}

#[guinea::test]
fn adding_ignores_blank_titles_and_assigns_stable_ids(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<Board>();
    actions.emit(Add("  Read the book  ".into()));
    actions.emit(Add("   ".into()));
    actions.emit(Add("Write a test".into()));
    let board = h.state::<Board>();
    assert_eq!(board.0.todos.len(), 2);
    assert_eq!(board.0.todos[0].title, "Read the book");
    assert_eq!(board.0.todos[0].id, 1);
    assert_eq!(board.0.todos[1].id, 2);
    assert_eq!(board.0.revision, 2);
}

#[guinea::test]
fn editing_completion_removal_and_undo_share_one_board(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<Board>();
    actions.emit(Add("first".into()));
    actions.emit(Add("second".into()));
    actions.emit(Rename(1, "renamed".into()));
    actions.emit(Toggle(1));
    assert_eq!(h.state::<Board>().0.completed(), 1);
    actions.emit(ClearCompleted);
    assert_eq!(h.state::<Board>().0.todos.len(), 1);
    actions.emit(Undo);
    assert_eq!(h.state::<Board>().0.todos[0].title, "renamed");
    assert!(h.state::<Board>().0.todos[0].done);
    actions.emit(Remove(2));
    actions.emit(Undo);
    assert_eq!(h.state::<Board>().0.todos.len(), 2);
}

#[guinea::test]
fn invalid_ids_and_blank_renames_do_not_create_changes(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<Board>();
    actions.emit(Add("keep".into()));
    actions.emit(Rename(1, " ".into()));
    actions.emit(Rename(99, "missing".into()));
    actions.emit(Toggle(99));
    actions.emit(Remove(99));
    assert_eq!(h.state::<Board>().0.revision, 1);
}

#[guinea::test]
fn undo_does_not_reuse_a_removed_task_id(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<Board>();
    actions.emit(Add("first".into()));
    actions.emit(Undo);
    actions.emit(Add("second".into()));
    assert_eq!(h.state::<Board>().0.todos[0].id, 2);
}

#[guinea::test]
fn only_the_latest_snapshot_is_saved_after_a_quiet_period(h: &mut Harness) {
    let writes = start(h);
    h.dispatch::<Board>().emit(Add("first".into()));
    h.settled();
    h.advance(Duration::from_millis(200));
    h.dispatch::<Board>().emit(Add("second".into()));
    h.settled();
    h.advance(Duration::from_millis(200));
    assert!(writes.borrow().is_empty());
    assert_eq!(*h.state::<SaveState>(), SaveState::Pending);
    h.advance(Duration::from_millis(100));
    assert_eq!(writes.borrow().len(), 1);
    assert_eq!(writes.borrow()[0].todos.len(), 2);
    assert_eq!(*h.state::<SaveState>(), SaveState::Saved(2));
}

#[guinea::test]
fn a_save_failure_is_visible_without_losing_the_board(h: &mut Harness) {
    h.app(|app| {
        configure(
            app,
            Repository::new(Snapshot::default(), |_| {
                anyhow::bail!("storage refused the write")
            }),
        )
    })
    .unwrap();
    h.dispatch::<Board>().emit(Add("not lost".into()));
    h.settled();
    h.advance(Duration::from_millis(300));
    assert_eq!(h.state::<Board>().0.todos.len(), 1);
    assert_eq!(
        *h.state::<SaveState>(),
        SaveState::Failed("storage refused the write".into())
    );
}

#[guinea::test]
fn the_event_bus_updates_the_journal_without_the_editor_knowing_its_address(h: &mut Harness) {
    start(h);
    h.dispatch::<Board>().emit(Add("tracked".into()));
    assert_eq!(h.state::<Activity>().0, ["Added: tracked"]);
    for index in 0..25 {
        h.dispatch::<Board>().emit(Add(format!("task {index}")));
    }
    assert_eq!(h.state::<Activity>().0.len(), 20);
}

#[guinea::test]
fn search_rejects_an_old_query_and_follows_new_board_snapshots(h: &mut Harness) {
    start(h);
    h.dispatch::<Board>().emit(Add("read".into()));
    h.dispatch::<Board>().emit(Add("write".into()));
    let page = h.child();
    page.install::<Search>(&()).unwrap();
    page.dispatch::<SearchResults>().emit(Query("read".into()));
    h.settled();
    h.advance(Duration::from_millis(75));
    page.dispatch::<SearchResults>().emit(Query("write".into()));
    h.settled();
    h.advance(Duration::from_millis(75));
    assert!(page.state::<SearchResults>().pending);
    h.advance(Duration::from_millis(75));
    assert_eq!(page.state::<SearchResults>().ids, [2]);
    h.dispatch::<Board>().emit(Add("write more tests".into()));
    h.settled();
    h.advance(Duration::from_millis(150));
    assert_eq!(page.state::<SearchResults>().ids, [2, 3]);
}

#[guinea::test]
fn a_sleeping_search_catches_up_from_the_feed_when_woken(h: &mut Harness) {
    start(h);
    let page = h.child();
    page.install::<Search>(&()).unwrap();
    h.settled();
    h.advance(Duration::from_millis(150));
    page.scope().sleep();
    h.dispatch::<Board>()
        .emit(Add("arrived while asleep".into()));
    h.advance(Duration::from_millis(150));
    assert!(page.state::<SearchResults>().ids.is_empty());
    page.scope().wake();
    h.settled();
    h.advance(Duration::from_millis(150));
    assert_eq!(page.state::<SearchResults>().ids, [1]);
}

#[guinea::test]
fn cancelling_an_import_stops_both_the_producer_and_the_stream(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<Board>();
    actions.emit(Import(sample_titles()));
    h.settled();
    h.advance(Duration::from_millis(100));
    assert_eq!(h.state::<Board>().0.todos.len(), 1);
    actions.emit(StopImport);
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Board>().0.todos.len(), 1);
}

#[guinea::test]
fn a_new_import_replaces_the_previous_stream(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<Board>();
    actions.emit(Import(vec!["old first".into(), "old second".into()]));
    h.settled();
    h.advance(Duration::from_millis(50));
    actions.emit(Import(vec!["replacement".into()]));
    h.settled();
    h.advance(Duration::from_millis(200));
    assert_eq!(
        h.state::<Board>()
            .0
            .todos
            .iter()
            .map(|todo| todo.title.as_str())
            .collect::<Vec<_>>(),
        ["replacement"]
    );
}

#[guinea::test]
fn syncing_uses_a_typed_request_to_an_unaddressed_service(h: &mut Harness) {
    start(h);
    h.dispatch::<Board>().emit(Add("sync me".into()));
    h.dispatch::<SyncState>().emit(SyncNow);
    h.settled();
    assert_eq!(*h.state::<SyncState>(), SyncState::Running);
    h.advance(Duration::from_millis(700));
    assert_eq!(*h.state::<SyncState>(), SyncState::Synced(1));
}

#[guinea::test]
fn cancelled_sync_cannot_overwrite_the_idle_state(h: &mut Harness) {
    start(h);
    let actions = h.dispatch::<SyncState>();
    actions.emit(SyncNow);
    h.settled();
    actions.emit(CancelSync);
    h.advance(Duration::from_secs(1));
    assert_eq!(*h.state::<SyncState>(), SyncState::Idle);
}

#[guinea::test]
fn application_state_and_timer_survive_a_page_being_removed(h: &mut Harness) {
    start(h);
    let page = h.child();
    page.install::<Search>(&()).unwrap();
    h.dispatch::<Board>().emit(Add("keep me".into()));
    page.leave();
    h.advance(Duration::from_secs(2));
    assert_eq!(h.state::<Board>().0.todos.len(), 1);
    assert_eq!(h.state::<Clock>().0, 2);
}

#[guinea::test]
fn reopening_restores_the_snapshot_and_continues_its_ids(h: &mut Harness) {
    let initial = Snapshot {
        todos: vec![Todo {
            id: 41,
            title: "already saved".into(),
            done: true,
        }],
        revision: 9,
    };
    h.app(move |app| configure(app, Repository::new(initial, |_| Ok(()))))
        .unwrap();
    h.dispatch::<Board>().emit(Add("new".into()));
    assert_eq!(h.state::<Board>().0.todos[1].id, 42);
    assert_eq!(h.state::<Board>().0.revision, 10);
}
