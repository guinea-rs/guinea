mod board;
mod persistence;
mod search;
mod storage;
mod sync;

pub use storage::{StorageConfig, StoragePlugin, startup};

pub use board::{Add, Board, ClearCompleted, Import, Remove, Rename, StopImport, Toggle, Undo};
pub use persistence::{SaveState, Storage};
pub use search::{Query, Search, SearchResults};
pub use sync::{CancelSync, SyncNow, SyncState};

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use guinea::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub id: u64,
    pub title: String,
    pub done: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub todos: Vec<Todo>,
    pub revision: u64,
}

impl Snapshot {
    pub fn completed(&self) -> usize {
        self.todos.iter().filter(|todo| todo.done).count()
    }

    pub fn summary(&self) -> String {
        format!("{} tasks, {} completed", self.todos.len(), self.completed())
    }
}

#[derive(Clone)]
pub struct Repository {
    initial: Snapshot,
    pending: Rc<RefCell<Snapshot>>,
    save: Rc<dyn Fn(Snapshot) -> anyhow::Result<()>>,
}

impl Repository {
    pub fn initial(&self) -> &Snapshot {
        &self.initial
    }

    pub fn new(initial: Snapshot, save: impl Fn(Snapshot) -> anyhow::Result<()> + 'static) -> Self {
        Self {
            pending: Rc::new(RefCell::new(initial.clone())),
            initial,
            save: Rc::new(save),
        }
    }
}

#[derive(Clone, Debug, guinea::Event)]
pub struct Changed {
    pub snapshot: Snapshot,
    pub description: String,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Activity(pub Vec<String>);

#[reducer]
fn activity(this: &mut Activity, entry: String) {
    this.0.push(entry);
    if this.0.len() > 20 {
        this.0.remove(0);
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Clock(pub u64);

#[reducer]
fn clock(this: &mut Clock, _: ()) {
    this.0 += 1;
}

struct Journal(Push<Activity>);

actor! {
    Journal {
        subscribes { Changed }
    }
}

#[handler]
fn changed(In(this, changed, _): In<Journal, Changed>) {
    this.0.send(changed.description);
}

feature! {
    pub TodoApp {
        exports { Board, Activity, Clock, SaveState, SyncState }
        feeds { Snapshot }
    }
}

#[installs]
fn todo_app(app: &mut FeatureBuilder, repository: Repository) -> anyhow::Result<TodoApp> {
    let snapshots = app.feed::<Snapshot>();
    snapshots.set(repository.initial.clone());
    let board = app
        .state::<Board>()
        .seed(Board(repository.initial.clone()))
        .driven_by(|push| board::Editing::new(repository.initial.clone(), push, snapshots.clone()))
        .0;
    let activity = app.state::<Activity>().driven_by(Journal).0;
    let save = app
        .state::<SaveState>()
        .driven_by(|push| Storage::new(repository, push))
        .0;
    app.spawn(sync::Server);
    let sync = app
        .state::<SyncState>()
        .driven_by(|push| sync::Client::new(snapshots.feed(), push))
        .0;
    let clock = app.state::<Clock>().plain();
    app.repeat(Duration::from_secs(1), move || clock.push(()));
    Ok(TodoApp(
        board,
        activity,
        clock,
        save,
        sync,
        snapshots.feed(),
    ))
}

pub fn configure(app: &mut FeatureBuilder, repository: Repository) -> anyhow::Result<()> {
    app.plugin::<StoragePlugin>(StorageConfig::Existing(repository))?;
    app.install::<TodoApp>()?;
    Ok(())
}

pub fn sample_titles() -> Vec<String> {
    [
        "Review the pull request",
        "Write a regression test",
        "Read the guinea book",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Active,
    Completed,
}

#[derive(Clone, Debug, PartialEq, guinea::Routable)]
pub enum Screen {
    Tasks { filter: Filter },
    Detail { id: u64 },
    Activity,
}

pub fn visible(snapshot: &Snapshot, search: &SearchResults, filter: Filter) -> Vec<Todo> {
    snapshot
        .todos
        .iter()
        .filter(|todo| {
            let matches_query = search.query.is_empty() || search.ids.contains(&todo.id);
            let matches_filter = match filter {
                Filter::All => true,
                Filter::Active => !todo.done,
                Filter::Completed => todo.done,
            };
            matches_query && matches_filter
        })
        .cloned()
        .collect()
}

pub fn status(save: &SaveState, sync: &SyncState) -> String {
    let saving = match save {
        SaveState::Clean => "No changes".into(),
        SaveState::Pending => "Saving…".into(),
        SaveState::Saved(revision) => format!("Saved revision {revision}"),
        SaveState::Failed(error) => format!("Save failed: {error}"),
    };
    let syncing = match sync {
        SyncState::Idle => "Offline demo".into(),
        SyncState::Running => "Syncing…".into(),
        SyncState::Synced(count) => format!("Synced {count} tasks"),
        SyncState::Failed(error) => format!("Sync failed: {error}"),
    };
    format!("{saving} · {syncing}")
}

#[cfg(test)]
mod declarations {
    #[test]
    fn the_application_contracts_are_complete_before_installation() {
        guinea::app::validate_declarations();
    }
}
