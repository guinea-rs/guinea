use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use todo_core::{Add, Board, Filter, Repository, Screen, Snapshot};

#[guinea::test]
fn deleting_from_the_interface_updates_the_board_and_the_journal(h: &mut Harness) {
    let window = h
        .app(|app| {
            todo_core::configure(app, Repository::new(Snapshot::default(), |_| Ok(())))?;
            Ok(todo_winui::window())
        })
        .unwrap();
    h.dispatch::<Board>().emit(Add("Delete me".into()));
    let mut ui = Mounted::<Screen>::from_window(h, window).unwrap();
    ui.settle();
    ui.click_text("Delete 1").settle();
    assert!(h.state::<Board>().0.todos.is_empty());
    ui.click_text("Activity").settle();
    assert!(ui.find_text("Removed task 1").is_some());
    ui.click_text("All").settle();
    assert_eq!(
        ui.route(),
        Screen::Tasks {
            filter: Filter::All
        }
    );
}

#[guinea::test]
fn detail_tools_are_owned_by_the_details_page(h: &mut Harness) {
    let window = h
        .app(|app| {
            todo_core::configure(app, Repository::new(Snapshot::default(), |_| Ok(())))?;
            Ok(todo_winui::window())
        })
        .unwrap();
    h.dispatch::<Board>().emit(Add("Inspect me".into()));
    let mut ui = Mounted::<Screen>::from_window(h, window).unwrap();
    ui.settle();
    assert!(ui.find_text("Save title").is_none());
    ui.click_text("Edit 1").settle();
    assert!(ui.find_text("Save title").is_some());
    ui.click_text("Back").settle();
    assert!(ui.find_text("Save title").is_none());
}
