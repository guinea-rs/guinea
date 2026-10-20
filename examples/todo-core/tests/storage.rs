use std::time::Duration;

use amethystate::StoreBuilder;
use guinea::app::{Harness, TestApp};
use todo_core::*;

#[guinea::test(iterations = 1)]
fn a_disk_plugin_restores_saved_edits(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("todo.json");
    h.app(|app| {
        app.plugin::<StoragePlugin>(StorageConfig::Disk(StoreBuilder::new(&path)))?;
        app.install::<TodoApp>()?;
        Ok(())
    })
    .unwrap();
    h.dispatch::<Board>().emit(Add("survives restart".into()));
    h.settled();
    assert_eq!(*h.state::<SaveState>(), SaveState::Pending);
    h.advance(Duration::from_millis(300));
    assert_eq!(*h.state::<SaveState>(), SaveState::Saved(1));
    let mut reopened = TestApp::new();
    let StoragePlugin(repository) = reopened
        .plugin::<StoragePlugin>(StorageConfig::Disk(StoreBuilder::new(&path)))
        .unwrap();
    assert_eq!(repository.initial().todos[0].title, "survives restart");
}

#[test]
fn exiting_flushes_edits_before_the_debounce_expires() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("todo.json");
    {
        let mut harness = Harness::new(0);
        harness
            .app(|app| {
                app.plugin::<StoragePlugin>(StorageConfig::Disk(StoreBuilder::new(&path)))?;
                app.install::<TodoApp>()?;
                Ok(())
            })
            .unwrap();
        harness
            .dispatch::<Board>()
            .emit(Add("not debounced yet".into()));
        assert_eq!(*harness.state::<SaveState>(), SaveState::Pending);
    }
    let mut reopened = TestApp::new();
    let StoragePlugin(repository) = reopened
        .plugin::<StoragePlugin>(StorageConfig::Disk(StoreBuilder::new(&path)))
        .unwrap();
    assert_eq!(repository.initial().todos[0].title, "not debounced yet");
}
