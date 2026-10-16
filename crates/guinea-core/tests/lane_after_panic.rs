use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use guinea_core::__private::Home;
use guinea_core::actor::{Addr, Handler, In};
use guinea_core::scope::ScopeTree;
use guinea_core::trace::{self, Trace};
use guinea_core::ui_thread::UiLoop;

#[derive(Debug)]
struct Saver(Rc<RefCell<Vec<u32>>>);

struct Save(u32);
struct Saved(u32);

guinea_macros::actor! {
    Saver {
        handlers {
            Save => { task Saved }
            Saved
        }
    }
}

impl Handler<Save> for Saver {
    fn handle(In(_, Save(n), cx): In<Self, Save>) {
        cx.task().exhaust().spawn(async move {
            if n == 0 {
                panic!("the save failed");
            }
            Saved(n)
        });
    }
}

impl Handler<Saved> for Saver {
    fn handle(In(this, Saved(n), _): In<Self, Saved>) {
        this.0.borrow_mut().push(n);
    }
}

#[test]
fn a_lane_whose_work_panicked_takes_work_again() {
    let ui = UiLoop::install();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _entered = runtime.enter();

    let tree = ScopeTree::new();
    let ends = Rc::new(RefCell::new(Vec::new()));
    let noted = ends.clone();
    let _observing = trace::sink_of(tree.scope()).expect("the tree is there").observe(move |trace| {
        if let Trace::Mark(record) = trace
            && ["settled", "cancelled"].contains(&record.point.kind())
        {
            noted.borrow_mut().push(record.point.kind());
        }
    });

    let saved = Rc::new(RefCell::new(Vec::new()));
    let saver = Addr::new(Saver(saved.clone()), &Home::new(tree.scope()));

    saver.send(Save(0));
    ui.run_until(|| false, Duration::from_millis(200));

    saver.send(Save(1));
    let answered = ui.run_until(|| !saved.borrow().is_empty(), Duration::from_secs(2));

    assert!(answered, "the lane stayed busy with work that had panicked: {:?}", saved.borrow());
    assert_eq!(*ends.borrow(), ["cancelled", "settled"], "the work that panicked ends in the trace");
}
