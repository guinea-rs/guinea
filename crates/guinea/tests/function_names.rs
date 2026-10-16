//! A function `#[reducer]`, `#[handler]` or `#[installs]` is written on may
//! have any name: nothing the macros write around its call shadows it.

use guinea::app::Harness;
use guinea::actor::ManagedActor;
use guinea::actor::event_bus::RpcRequest;
use guinea::prelude::*;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Model {
    pub count: u32,
}

#[reducer]
fn update(this: &mut Model, by: u32) {
    this.count += by;
}

#[test]
fn a_reducer_named_update_reduces() {
    let mut model = Model::default();

    model.reduce(2);

    assert_eq!(model.count, 2);
}

pub struct Line;

pub struct Again;

pub struct Later;

#[derive(Clone, Debug, guinea::Request)]
#[request(reply = u32)]
pub struct Count;

#[derive(Debug, Default)]
pub struct Log;

actor! {
    Log {
        handlers { Line, Again, Later, RpcRequest<Count> }
    }
}

#[handler]
fn msg(_this: &mut Log, _: Line) {}

#[handler]
fn cx(_this: &mut Log, _: Again, cx: Cx) {
    let _ = cx;
}

#[handler]
async fn actx(_cx: AsyncContext<Log>, _: Later) {}

#[handler]
async fn payload(_cx: AsyncContext<Log>, _: Count) -> u32 {
    1
}

#[test]
fn a_handler_may_be_named_as_what_its_call_is_handed() {
    let shape = <Log as ManagedActor>::SHAPE;

    assert_eq!(shape.handles.len(), 4);
    assert!(shape.handles.iter().all(|handles| handles.declared.is_some()));
}

pub struct Settings;

feature! {
    pub Configured {}
}

#[installs]
fn params(cx: &Install<Settings>) -> anyhow::Result<Configured> {
    let _ = cx.params();
    Ok(Configured)
}

feature! {
    pub Plain {}
}

mod plain {
    use super::*;

    #[installs]
    fn cx(cx: &FeatureInitContext) -> anyhow::Result<Plain> {
        let _ = cx;
        Ok(Plain)
    }
}

#[test]
fn an_installer_may_be_named_as_what_its_call_is_handed() {
    let h = Harness::new(0);

    h.install::<Configured>(&Settings).unwrap();
    h.install::<Plain>(&()).unwrap();
}
