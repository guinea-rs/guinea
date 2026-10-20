use std::time::Duration;

use guinea::prelude::*;

use crate::Snapshot;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct SearchResults {
    pub query: String,
    pub ids: Vec<u64>,
    pub pending: bool,
}

#[derive(Clone)]
pub enum Update {
    Pending(String),
    Found(String, Vec<u64>),
}

#[reducer]
fn search_results(this: &mut SearchResults, update: Update) {
    match update {
        Update::Pending(query) => {
            this.query = query;
            this.pending = true;
        }
        Update::Found(query, ids) => {
            this.query = query;
            this.ids = ids;
            this.pending = false;
        }
    }
}

pub struct Query(pub String);
struct Reindex;

struct Searching {
    query: String,
    snapshots: Feed<Snapshot>,
    push: Push<SearchResults>,
}

actor! {
    Searching {
        handlers { Query, Reindex }
    }
}

impl Searching {
    fn input(&self) -> (String, Snapshot) {
        (
            self.query.clone(),
            self.snapshots.get().as_deref().cloned().unwrap_or_default(),
        )
    }
}

async fn find(query: String, snapshot: Snapshot) -> Update {
    tokio::time::sleep(Duration::from_millis(150)).await;
    let needle = query.to_lowercase();
    let ids = snapshot
        .todos
        .into_iter()
        .filter(|todo| todo.title.to_lowercase().contains(&needle))
        .map(|todo| todo.id)
        .collect();
    Update::Found(query, ids)
}

#[handler]
fn query(In(this, Query(query), cx): In<Searching, Query>) {
    this.query = query;
    let (query, snapshot) = this.input();
    this.push.send(Update::Pending(query.clone()));
    cx.task()
        .latest()
        .pushing(this.push)
        .spawn(find(query, snapshot));
}

#[handler]
fn reindex(In(this, _, cx): In<Searching, Reindex>) {
    let (query, snapshot) = this.input();
    this.push.send(Update::Pending(query.clone()));
    cx.task()
        .latest()
        .pushing(this.push)
        .spawn(find(query, snapshot));
}

feature! {
    pub Search {
        exports { SearchResults }
    }
}

#[installs]
fn search(cx: &Install, snapshots: Feed<Snapshot>) -> anyhow::Result<Search> {
    let (results, actor) = cx.state::<SearchResults>().driven_by(|push| Searching {
        query: String::new(),
        snapshots: snapshots.clone(),
        push,
    });
    cx.follow(snapshots, move |_| actor.send(Reindex));
    Ok(Search(results))
}
