use std::time::Duration;

use guinea::actor::event_bus::RpcRequest;
use guinea::prelude::*;

use crate::Snapshot;

#[derive(Clone, Debug, guinea::Request)]
#[request(reply = usize)]
struct Upload(Snapshot);

pub(crate) struct Server;

actor! {
    Server {
        subscribes { RpcRequest<Upload> }
    }
}

#[handler]
fn upload(In(_, Upload(snapshot), _): In<Server, Upload>) -> Reply<usize> {
    Reply::later(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        snapshot.todos.len()
    })
}

#[derive(Default, Clone, Debug, PartialEq)]
pub enum SyncState {
    #[default]
    Idle,
    Running,
    Synced(usize),
    Failed(String),
}

#[reducer]
fn sync_state(this: &mut SyncState, state: SyncState) {
    *this = state;
}

pub struct SyncNow;
pub struct CancelSync;

pub(crate) struct Client {
    snapshots: Feed<Snapshot>,
    push: Push<SyncState>,
}

impl Client {
    pub(crate) fn new(snapshots: Feed<Snapshot>, push: Push<SyncState>) -> Self {
        Self { snapshots, push }
    }
}

actor! {
    Client {
        handlers { SyncNow, CancelSync }
    }
}

#[handler]
fn sync_now(In(this, _, cx): In<Client, SyncNow>) {
    let snapshot = this.snapshots.get().as_deref().cloned().unwrap_or_default();
    let remote = cx.async_ctx();
    this.push.send(SyncState::Running);
    cx.task().latest().pushing(this.push).spawn(async move {
        match remote
            .request(Upload(snapshot), Duration::from_secs(2))
            .await
        {
            Ok(count) => SyncState::Synced(count),
            Err(error) => SyncState::Failed(format!("{:?}", error.why)),
        }
    });
}

#[handler]
fn cancel_sync(In(this, _, cx): In<Client, CancelSync>) {
    cx.task().lane::<SyncState>().cancel();
    this.push.send(SyncState::Idle);
}
