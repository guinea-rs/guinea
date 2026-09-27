//! The actor, no longer generic over a port.
//!
//! It used to be `ProcessActor<P: ProcessesPort>` because the way back into
//! the reducer arrived as an unnamed callback and the trait was what named it.
//! `driven_by` hands over a `Push<Processes>` instead - the direction is in
//! the type - so the generic, the trait and the blanket impl behind it are all
//! gone.

use std::rc::Rc;

use guinea::core::actor::event_bus::EventBus;
use guinea::prelude::*;

use crate::events::ProcessKilled;

use super::contracts::{Kill, Listed, Processes, Refresh};

#[derive(Debug)]
pub struct ProcessActor {
    push: Push<Processes>,
    items: Vec<String>,
    event_bus: Rc<EventBus>,
}

impl ProcessActor {
    pub fn new(_context: String, push: Push<Processes>, event_bus: Rc<EventBus>) -> Self {
        Self {
            push,
            items: Vec::new(),
            event_bus,
        }
    }

    fn publish(&self) {
        self.push.send(Listed::Items(self.items.clone()));
    }
}

actor! {
    ProcessActor {
        handlers  { Kill, Refresh }
        publishes { ProcessKilled }
    }
}

#[handler]
fn kill(this: &mut ProcessActor, Kill(pid): Kill, cx: Cx) {
    let needle = format!("(pid {pid})");
    let killed = this.items.iter().find(|row| row.ends_with(&needle)).cloned();
    this.items.retain(|row| !row.ends_with(&needle));
    this.publish();

    if let Some(name) = killed {
        // Window-local: only this window's `TabsLayout` hears it.
        cx.publish_local(&this.event_bus, ProcessKilled { name: name.clone() });
        // Process-wide: every window's `TabsLayout` hears it, including
        // ones opened after this event fires.
        cx.publish(ProcessKilled { name });
    }
}

#[handler]
fn refresh(this: &mut ProcessActor, _: Refresh) {
    this.items = vec![
        "systemd (pid 1)".to_string(),
        "sshd (pid 42)".to_string(),
        "bash (pid 512)".to_string(),
        "cargo (pid 900)".to_string(),
    ];
    this.publish();
}
