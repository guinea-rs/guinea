use std::time::Duration;

use guinea::prelude::*;

use crate::{Changed, Repository, Snapshot};

#[derive(Default, Clone, Debug, PartialEq)]
pub enum SaveState {
    #[default]
    Clean,
    Pending,
    Saved(u64),
    Failed(String),
}

#[reducer]
fn save_state(this: &mut SaveState, state: SaveState) {
    *this = state;
}

pub struct Storage {
    repository: Repository,
    push: Push<SaveState>,
}

impl Storage {
    pub(crate) fn new(repository: Repository, push: Push<SaveState>) -> Self {
        Self { repository, push }
    }
}

struct Save(Snapshot);

actor! {
    Storage {
        subscribes { Changed => { task Save } }
        handlers {
            Save
        }
    }
}

#[handler]
fn changed(In(this, changed, cx): In<Storage, Changed>) {
    *this.repository.pending.borrow_mut() = changed.snapshot.clone();
    this.push.send(SaveState::Pending);
    cx.task().latest().spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Save(changed.snapshot)
    });
}

#[handler]
fn save(In(this, Save(snapshot), _): In<Storage, Save>) {
    let revision = snapshot.revision;
    let state = match (this.repository.save)(snapshot) {
        Ok(()) => SaveState::Saved(revision),
        Err(error) => SaveState::Failed(error.to_string()),
    };
    this.push.send(state);
}
