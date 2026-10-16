//! An actor that sends on and spawns, for the tests of what a handler may do.

use super::In;
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
    fn handle(In(this, _, cx): In<Self, First>) {
        this.log.borrow_mut().push("first");
        cx.send(Second);
    }
}

impl Handler<Second> for Chain {
    fn handle(In(this, _, _): In<Self, Second>) {
        this.log.borrow_mut().push("second");
    }
}
