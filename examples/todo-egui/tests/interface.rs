use guinea::app::Harness;
use guinea::eframe::harness::Mounted;
use todo_core::{Add, Board, Repository, Snapshot, Toggle};

#[guinea::test]
fn the_same_board_is_reached_through_all_filters(h: &mut Harness) {
    let window = h
        .app(|app| {
            todo_core::configure(app, Repository::new(Snapshot::default(), |_| Ok(())))?;
            Ok(todo_egui::window())
        })
        .unwrap();
    h.dispatch::<Board>().emit(Add("A real task".into()));
    let mut ui = Mounted::window(h, window);
    ui.settle();
    assert!(ui.has_text("A real task"));
    ui.click_text("Activity");
    assert!(ui.has_text("Added: A real task"));
    ui.click_text("Completed");
    assert!(!ui.has_text("A real task"));
    h.dispatch::<Board>().emit(Toggle(1));
    ui.settle();
    assert!(ui.has_text("A real task"));
    ui.click_text("Active");
    assert!(!ui.has_text("A real task"));
    ui.click_text("All");
    assert!(ui.has_text("A real task"));
}

#[guinea::test]
fn deleting_from_the_interface_updates_the_model_and_the_log(h: &mut Harness) {
    let window = h
        .app(|app| {
            todo_core::configure(app, Repository::new(Snapshot::default(), |_| Ok(())))?;
            Ok(todo_egui::window())
        })
        .unwrap();
    h.dispatch::<Board>().emit(Add("Delete me".into()));
    let mut ui = Mounted::window(h, window);
    ui.click_text("Delete 1");
    assert!(h.state::<Board>().0.todos.is_empty());
    ui.click_text("Activity");
    assert!(ui.has_text("Removed task 1"));
}
