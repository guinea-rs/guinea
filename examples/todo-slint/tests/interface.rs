use guinea::app::Harness;
use slint::{ComponentHandle, Model};
use todo_core::{Board, Repository, Snapshot};
use todo_slint::{AppWindow, TodoUi};

#[guinea::test(iterations = 1)]
fn declarative_callbacks_drive_the_shared_board(h: &mut Harness) {
    i_slint_backend_testing::init_no_event_loop();
    h.app(|app| todo_core::configure(app, Repository::new(Snapshot::default(), |_| Ok(()))))
        .unwrap();
    let component = AppWindow::new().unwrap();
    let root = h.child();
    let _opened = todo_slint::window_for(component.clone_strong())
        .open(root.scope())
        .unwrap();
    component.global::<TodoUi>().invoke_add("from Slint".into());
    h.settled();
    assert_eq!(h.state::<Board>().0.todos[0].title, "from Slint");
    assert_eq!(component.global::<TodoUi>().get_todos().row_count(), 1);
    component.global::<TodoUi>().invoke_toggle("1".into());
    assert!(h.state::<Board>().0.todos[0].done);
    component.global::<TodoUi>().invoke_remove("1".into());
    h.settled();
    assert!(h.state::<Board>().0.todos.is_empty());
    assert_eq!(component.global::<TodoUi>().get_todos().row_count(), 0);
}
