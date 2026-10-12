//! What `actor!` reads from a declaration.

use std::marker::PhantomData;

use guinea_core::actor::ManagedActor;
use guinea_macros::{actor, handler};

pub struct Load;

pub struct Loaded<K, V>(PhantomData<(K, V)>);

pub struct Evicted<K, V>(PhantomData<(K, V)>);

#[derive(Debug)]
pub struct Cache;

actor! {
    Cache {
        handlers {
            Load => { send Loaded<String, u32>, task Loaded<String, u32> },
            Loaded<String, u32>,
        }
        publishes { Evicted<String, u32> }
    }
}

#[handler]
fn load(_this: &mut Cache, _: Load) {}

#[handler]
fn loaded(_this: &mut Cache, _: Loaded<String, u32>) {}

pub mod global {
    pub struct Refreshed;
}

pub mod local {
    pub struct Refreshed;
}

pub struct Refresh;

pub struct Announced;

#[derive(Debug)]
pub struct Poller;

actor! {
    Poller {
        handlers { Refresh => { emit global::Refreshed, emit local::Refreshed, emit global Announced } }
    }
}

#[handler]
fn refresh(_this: &mut Poller, _: Refresh) {}

#[test]
fn an_emit_target_in_a_module_named_global_or_local_keeps_its_path() {
    let edges = <Poller as ManagedActor>::SHAPE.handles[0].edges.expect("declared");
    let targets: Vec<&str> = edges.iter().map(|edge| (edge.target)()).collect();

    assert!(targets[0].ends_with("global::Refreshed"), "{targets:?}");
    assert!(targets[1].ends_with("local::Refreshed"), "{targets:?}");
    assert!(targets[2].ends_with("::Announced"), "{targets:?}");
}

#[test]
fn a_message_with_two_type_arguments_is_one_message() {
    let shape = <Cache as ManagedActor>::SHAPE;

    assert_eq!(shape.handles.len(), 2);
    assert!((shape.handles[1].message)().ends_with("Loaded<alloc::string::String, u32>"));
    let edges = shape.handles[0].edges.expect("declared");
    assert_eq!(edges.len(), 2);
    assert!((edges[0].target)().ends_with("Loaded<alloc::string::String, u32>"));
    assert_eq!(shape.publishes.len(), 1);
}
