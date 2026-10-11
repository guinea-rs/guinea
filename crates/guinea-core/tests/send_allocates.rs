//! Sending is on every path, so it costs no allocation where it can avoid
//! one.
//!
//! A process of its own, because the allocator is process-wide.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use guinea_core::actor::{Addr, Cx, Handler, Home};
use guinea_core::scope::ScopeTree;

struct Counting;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.with(|allocated| allocated.set(allocated.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn allocations(during: impl FnOnce()) -> usize {
    let before = ALLOCATED.with(Cell::get);
    during();
    ALLOCATED.with(Cell::get) - before
}

#[derive(Debug)]
struct Sink(u64);

struct Payload([u8; 64]);

guinea_macros::actor! {
    Sink {
        handlers { Payload }
    }
}

impl Handler<Payload> for Sink {
    fn handle(&mut self, Payload(bytes): Payload, _cx: Cx<Self, Payload>) {
        self.0 += bytes.len() as u64;
    }
}

#[derive(Debug)]
struct Echoes(Rc<RefCell<Vec<u32>>>);

struct Echo(u32);

guinea_macros::actor! {
    Echoes {
        handlers { Echo }
    }
}

impl Handler<Echo> for Echoes {
    fn handle(&mut self, Echo(left): Echo, cx: Cx<Self, Echo>) {
        if left > 0 {
            cx.addr().send(Echo(left - 1));
        }
        self.0.borrow_mut().push(left);
    }
}

#[test]
fn a_message_to_an_actor_that_is_not_busy_is_handled_without_allocating() {
    let tree = ScopeTree::new();
    let addr = Addr::new(Sink(0), &Home::new(tree.scope()));
    addr.send(Payload([0; 64]));

    let allocated = allocations(|| {
        for _ in 0..100 {
            addr.send(Payload([0; 64]));
        }
    });

    assert_eq!(allocated, 0);
}

#[test]
fn a_message_an_actor_sends_itself_waits_its_turn_and_is_handled() {
    let tree = ScopeTree::new();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let addr = Addr::new(Echoes(heard.clone()), &Home::new(tree.scope()));

    addr.send(Echo(3));

    assert_eq!(*heard.borrow(), [3, 2, 1, 0]);
}
