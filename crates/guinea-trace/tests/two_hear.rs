use std::cell::Cell;
use std::rc::Rc;

use guinea_trace::{Point, is_observed, mark, observe};

#[test]
fn two_observers_of_a_thread_both_hear_and_one_leaving_leaves_the_other() {
    let first = Rc::new(Cell::new(0));
    let second = Rc::new(Cell::new(0));
    let one = first.clone();
    let two = second.clone();

    let hearing_first = observe(move |_| one.set(one.get() + 1));
    let hearing_second = observe(move |_| two.set(two.get() + 1));
    mark(|| Point::Push { reducer: "Both" });
    drop(hearing_first);
    mark(|| Point::Push { reducer: "Second" });
    let still = is_observed();
    drop(hearing_second);

    assert_eq!(
        (first.get(), second.get(), still, is_observed()),
        (1, 2, true, false)
    );
}
