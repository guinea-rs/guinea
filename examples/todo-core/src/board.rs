use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use guinea::actor::Stream;
use guinea::prelude::*;
use tokio::sync::mpsc;

use crate::{Changed, Snapshot, Todo};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Board(pub Snapshot);

#[reducer]
fn board(this: &mut Board, snapshot: Snapshot) {
    this.0 = snapshot;
}

pub struct Add(pub String);
pub struct Toggle(pub u64);
pub struct Rename(pub u64, pub String);
pub struct Remove(pub u64);
pub struct ClearCompleted;
pub struct Undo;
pub struct Import(pub Vec<String>);
pub struct StopImport;
struct Imported(String);
struct ImportProducer;

pub(crate) struct Editing {
    snapshot: Snapshot,
    previous: Vec<Vec<Todo>>,
    next_id: u64,
    push: Push<Board>,
    snapshots: FeedWriter<Snapshot>,
}

impl Editing {
    pub(crate) fn new(
        snapshot: Snapshot,
        push: Push<Board>,
        snapshots: FeedWriter<Snapshot>,
    ) -> Self {
        let next_id = snapshot.todos.iter().map(|todo| todo.id).max().unwrap_or(0) + 1;
        Self {
            snapshot,
            previous: Vec::new(),
            next_id,
            push,
            snapshots,
        }
    }

    fn remember(&mut self) {
        self.previous.push(self.snapshot.todos.clone());
        if self.previous.len() > 20 {
            self.previous.remove(0);
        }
    }

    fn commit(&mut self, description: String) -> Changed {
        self.snapshot.revision += 1;
        self.push.send(self.snapshot.clone());
        self.snapshots.set(self.snapshot.clone());
        Changed {
            snapshot: self.snapshot.clone(),
            description,
        }
    }

    fn add(&mut self, title: String) -> Option<Changed> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return None;
        }
        self.remember();
        self.snapshot.todos.push(Todo {
            id: self.next_id,
            title: title.clone(),
            done: false,
        });
        self.next_id += 1;
        Some(self.commit(format!("Added: {title}")))
    }
}

actor! {
    Editing {
        handlers {
            Add,
            Toggle,
            Rename,
            Remove,
            ClearCompleted,
            Undo,
            Import => { task Imported },
            Imported,
            StopImport
        }
        publishes { Changed }
    }
}

#[handler]
fn add(In(this, Add(title), cx): In<Editing, Add>) {
    if let Some(changed) = this.add(title) {
        cx.publish(changed);
    }
}

#[handler]
fn toggle(In(this, Toggle(id), cx): In<Editing, Toggle>) {
    if let Some(index) = this.snapshot.todos.iter().position(|todo| todo.id == id) {
        this.remember();
        this.snapshot.todos[index].done = !this.snapshot.todos[index].done;
        cx.publish(this.commit(format!("Toggled task {id}")));
    }
}

#[handler]
fn rename(In(this, Rename(id, title), cx): In<Editing, Rename>) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    if let Some(index) = this
        .snapshot
        .todos
        .iter()
        .position(|todo| todo.id == id && todo.title != title)
    {
        this.remember();
        this.snapshot.todos[index].title = title.to_owned();
        cx.publish(this.commit(format!("Renamed task {id}")));
    }
}

#[handler]
fn remove(In(this, Remove(id), cx): In<Editing, Remove>) {
    if this.snapshot.todos.iter().any(|todo| todo.id == id) {
        this.remember();
        this.snapshot.todos.retain(|todo| todo.id != id);
        cx.publish(this.commit(format!("Removed task {id}")));
    }
}

#[handler]
fn clear_completed(In(this, _, cx): In<Editing, ClearCompleted>) {
    if this.snapshot.completed() > 0 {
        this.remember();
        this.snapshot.todos.retain(|todo| !todo.done);
        cx.publish(this.commit("Cleared completed tasks".into()));
    }
}

#[handler]
fn undo(In(this, _, cx): In<Editing, Undo>) {
    if let Some(previous) = this.previous.pop() {
        this.snapshot.todos = previous;
        cx.publish(this.commit("Undid the last change".into()));
    }
}

struct ImportStream(mpsc::Receiver<String>);

impl Stream for ImportStream {
    type Item = String;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        self.0.poll_recv(cx)
    }
}

#[handler]
fn import(In(_, Import(titles), cx): In<Editing, Import>) {
    let (sender, receiver) = mpsc::channel(2);
    cx.task()
        .lane::<ImportProducer>()
        .latest()
        .detached()
        .spawn(async move {
            for title in titles {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if sender.send(title).await.is_err() {
                    break;
                }
            }
        });
    cx.task().latest().pour(ImportStream(receiver), Imported);
}

#[handler]
fn imported(In(this, Imported(title), cx): In<Editing, Imported>) {
    if let Some(changed) = this.add(title) {
        cx.publish(changed);
    }
}

#[handler]
fn stop_import(In(_, _, cx): In<Editing, StopImport>) {
    cx.task().lane::<ImportProducer>().cancel();
    cx.task().lane::<Imported>().cancel();
}
