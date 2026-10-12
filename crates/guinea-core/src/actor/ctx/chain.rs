//! An actor that sends on and spawns, for the tests of what a handler may do.

use super::Cx;
use crate::actor::traits::Handler;
use std::cell::RefCell;
use std::rc::Rc;

pub(super) struct First;
pub(super) struct Second;

#[derive(Debug)]
pub(super) struct Chain {
    pub(super) log: Rc<RefCell<Vec<&'static str>>>,
}

guinea_macros::actor! {
    Chain {
        handlers {
            First => { send Second, task Second }
            Second
        }
    }
}

impl Handler<First> for Chain {
    fn handle(&mut self, _: First, cx: Cx<Self, First>) {
        self.log.borrow_mut().push("first");
        cx.send(Second);
    }
}

impl Handler<Second> for Chain {
    fn handle(&mut self, _: Second, _cx: Cx<Self, Second>) {
        self.log.borrow_mut().push("second");
    }
}
