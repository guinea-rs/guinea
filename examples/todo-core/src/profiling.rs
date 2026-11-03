//! A repeatable user session over the same application core as the GUI frontends.
//! The harness drives actions and debounce deadlines; application code is unchanged.
use std::{path::Path, time::Duration};

use amethystate::StoreBuilder;
use guinea::app::{Harness, TestApp};
use guinea::prelude::*;

use crate::*;

#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub tasks: usize,
    pub revision: u64,
    pub matches: usize,
    pub journal_entries: usize,
}

pub fn session(
    app: &mut Harness,
    path: &Path,
    tasks: usize,
    mut checkpoint: impl FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<Report> {
    anyhow::ensure!(tasks >= 64, "the session needs at least 64 tasks");
    app.app(|app| {
        app.plugin::<StoragePlugin>(StorageConfig::Disk(StoreBuilder::new(path)))?;
        app.install::<TodoApp>()?;
        Ok(())
    })?;
    checkpoint("Application installed")?;

    let board = app.dispatch::<Board>();
    let titles = [
        "Review release checklist",
        "Write regression test",
        "Read design notes",
    ];
    for index in 0..tasks {
        board.emit(Add(format!(
            "{} #{}",
            titles[index % titles.len()],
            index + 1
        )));
        if (index + 1) % 64 == 0 {
            app.settled();
            checkpoint("Editing the board")?;
        }
    }
    app.settled();
    app.advance(Duration::from_millis(300));
    anyhow::ensure!(
        *app.state::<SaveState>() == SaveState::Saved(tasks as u64),
        "initial board was not saved"
    );
    checkpoint("Initial board saved")?;

    for id in 1..=32 {
        board.emit(Toggle(id));
    }
    for id in 1..=16 {
        board.emit(Rename(id, format!("Review release checklist #{id}")));
    }
    checkpoint("Tasks completed and renamed")?;

    let page = app.child();
    page.install::<Search>(&())?;
    let search = page.dispatch::<SearchResults>();
    for query in ["r", "regr", "release"] {
        search.emit(Query(query.into()));
    }
    app.settled();
    app.advance(Duration::from_millis(150));
    let matches = {
        let result = page.state::<SearchResults>();
        anyhow::ensure!(
            !result.pending && result.query == "release",
            "latest search did not finish"
        );
        result.ids.len()
    };
    checkpoint("Latest search completed")?;

    // Search follows the application's feed when a different actor changes the board.
    board.emit(ClearCompleted);
    app.settled();
    app.advance(Duration::from_millis(150));
    anyhow::ensure!(
        app.state::<Board>().0.todos.len() == tasks - 32,
        "completed tasks remain on the board"
    );
    board.emit(Undo);
    app.settled();
    app.advance(Duration::from_millis(300));
    anyhow::ensure!(
        app.state::<Board>().0.todos.len() == tasks,
        "undo did not restore the board"
    );
    checkpoint("Undo saved and search refreshed")?;

    let scope = page.scope();
    page.leave();
    app.settled();
    anyhow::ensure!(!scope.is_alive(), "search page remained alive");
    checkpoint("Search page closed")?;

    let snapshot = &app.state::<Board>().0;
    anyhow::ensure!(
        *app.state::<SaveState>() == SaveState::Saved(snapshot.revision),
        "final changes were not saved"
    );
    Ok(Report {
        tasks: snapshot.todos.len(),
        revision: snapshot.revision,
        matches,
        journal_entries: app.state::<Activity>().0.len(),
    })
}

/// Reopen through the real storage plugin, after the application's teardown.
pub fn verify_saved(path: &Path, report: &Report) -> anyhow::Result<()> {
    let mut app = TestApp::new();
    let StoragePlugin(repository) =
        app.plugin::<StoragePlugin>(StorageConfig::Disk(StoreBuilder::new(path)))?;
    anyhow::ensure!(
        repository.initial().todos.len() == report.tasks,
        "task count changed after reopening storage"
    );
    anyhow::ensure!(
        repository.initial().revision == report.revision,
        "saved revision changed after reopening storage"
    );
    anyhow::ensure!(
        repository.initial().completed() == 32,
        "completion state was not restored"
    );
    Ok(())
}
