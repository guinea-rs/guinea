//! Publishing is on every path, so it costs no allocation of its own: not
//! for the list of who hears it, not for the copy each of them is handed.
//!
//! A process of its own, because the allocator is process-wide.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::rc::Rc;

use guinea_core::BusSubscription;
use guinea_core::actor::event_bus::{Event, EventBus};
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

#[derive(Clone)]
struct Saved([u8; 256]);

impl Event for Saved {}

#[derive(Debug)]
struct Sink(u64);

guinea_macros::actor! {
    Sink {
        handlers { Saved }
    }
}

impl Handler<Saved> for Sink {
    fn handle(&mut self, Saved(bytes): Saved, _cx: Cx<Self, Saved>) {
        self.0 += black_box(bytes).len() as u64;
    }
}

#[test]
fn an_event_heard_by_callbacks_and_actors_is_published_without_allocating() {
    let tree = ScopeTree::new();
    let home = Home::new(tree.scope());
    let bus = Rc::new(EventBus::new());

    let heard = Rc::new(Cell::new(0));
    let mut subscriptions: Vec<BusSubscription> = (0..4)
        .map(|_| {
            let heard = heard.clone();
            bus.subscribe_fn(move |Saved(bytes): Saved| heard.set(heard.get() + black_box(bytes).len()))
        })
        .collect();
    subscriptions.extend((0..4).map(|_| bus.subscribe::<Sink, Saved>(Addr::new(Sink(0), &home))));
    bus.publish(Saved([0; 256]));

    let allocated = allocations(|| {
        for _ in 0..100 {
            bus.publish(Saved([0; 256]));
        }
    });

    assert_eq!((allocated, heard.get()), (0, 101 * 4 * 256));
}
