//! What `actor!` reads from a declaration.

use std::marker::PhantomData;

use guinea_core::actor::{In, ManagedActor};
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
fn load(_: In<Cache, Load>) {}

#[handler]
fn loaded(_: In<Cache, Loaded<String, u32>>) {}

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

#[test]
fn a_send_keeps_its_call_site_and_handling_keeps_its_declaration() {
    use guinea_core::{
        actor::{Addr, Handler},
        scope::ScopeTree,
        trace::{self, Point, Trace},
    };
    use std::{cell::RefCell, rc::Rc};
    let tree = ScopeTree::new();
    let sink = trace::sink_of(tree.scope()).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let output = seen.clone();
    let _observer = sink.observe(move |trace| {
        if let Trace::Begin(record) | Trace::Mark(record) = trace {
            output.borrow_mut().push(record.clone());
        }
    });
    let _current = trace::record_into(&sink);
    let addr = Addr::new(Cache, &guinea_core::__private::Home::new(tree.scope()));
    let sent_line = line!() + 1;
    addr.send(Load);
    let seen = seen.borrow();
    let sent = seen
        .iter()
        .find(|record| matches!(record.point, Point::Send { .. }))
        .unwrap();
    assert_eq!(
        (sent.location.file, sent.location.line),
        (file!(), sent_line)
    );
    let handled = seen
        .iter()
        .find(|record| matches!(record.point, Point::Handle { .. }))
        .unwrap();
    let declared = <Cache as Handler<Load>>::DECLARED.unwrap();
    assert_eq!(
        handled.location,
        trace::CodeLocation {
            file: declared.file,
            line: declared.line,
            column: declared.column,
            crate_dir: Some(declared.crate_dir),
        }
    );
}
