use std::rc::Rc;

use guinea_core::actor::event_bus::{EventBus, HeardBy, Listener, Listening};
use guinea_core::actor::{Addr, Cx, Handler, Home, UiThreadToken};
use guinea_core::scope::ScopeTree;

#[derive(Clone, Debug, guinea_macros::Event)]
struct Report;

#[derive(Clone, Debug, guinea_macros::Event)]
struct Pressed;

#[derive(Debug)]
struct Processes;

guinea_macros::actor! {
    Processes {
        handlers { Report }
    }
}

impl Handler<Report> for Processes {
    fn handle(&mut self, _: Report, _cx: Cx<Self, Report>) {}
}

#[test]
fn a_bus_says_who_hears_each_event_actors_by_id_and_callbacks_by_where_they_subscribed() {
    let bus = Rc::new(EventBus::new());
    let tree = ScopeTree::new();
    let home = Home::new(tree.scope(), UiThreadToken::dangerously_create_token_unchecked(), None);
    let processes = Addr::new(Processes, &home);

    let _actor = bus.subscribe::<Processes, Report>(processes.clone());
    let place = std::panic::Location::caller();
    let (_callback, subscribed_at) = (bus.subscribe_fn(|_: Report| {}), line!());
    let _pressed = bus.subscribe_fn(|_: Pressed| {});

    let mut listening = bus.listeners();
    listening.sort_by_key(|listening| listening.event);
    let report = listening
        .iter()
        .find(|listening| listening.event.ends_with("Report"))
        .expect("Report is listed");

    assert_eq!(listening.len(), 2, "{listening:#?}");
    assert_eq!(
        report.listeners[0],
        Listener {
            by: HeardBy::Actor {
                name: guinea_core::actor::short_type_name::<Processes>(),
                id: processes.id(),
            },
            answers: false,
            asleep: false,
        }
    );
    match report.listeners[1].by {
        HeardBy::Callback(at) => {
            assert_eq!(at.file(), place.file());
            assert_eq!(at.line(), subscribed_at, "where the test subscribed, not inside the bus");
        }
        ref other => panic!("a callback, not {other:?}"),
    }
    assert!(matches!(listening.as_slice(), [Listening { .. }, Listening { .. }]));

    processes.dispose();
}
