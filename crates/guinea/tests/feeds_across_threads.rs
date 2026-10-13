//! A feed written on a thread of its own and followed on the UI thread, with
//! a real UI loop rather than the harness's one thread: a binary of its own,
//! since a `UiLoop` makes the thread that installs it the process's UI thread.

use std::cell::RefCell;
use std::rc::Rc;

use guinea::core::scope::ScopeTree;
use guinea::core::ui_thread::UiLoop;

#[test]
fn writes_from_another_thread_reach_the_ui_thread_in_one_hop_and_stop_with_their_scope() {
    let ui = UiLoop::install();
    let tree = ScopeTree::new();
    let owner = tree.scope().child();
    let reader = tree.scope().child();

    let writer = owner.section().feed::<u32>();
    let seen = Rc::new(RefCell::new(Vec::new()));
    writer.feed().follow(reader, {
        let seen = seen.clone();
        move |value| seen.borrow_mut().push(*value)
    });

    let burst = writer.clone();
    std::thread::spawn(move || (1..=1000).for_each(|value| burst.set(value)))
        .join()
        .unwrap();

    assert_eq!(ui.run(), 1, "a burst of writes is one hop to the UI thread");
    assert_eq!(*seen.borrow(), [1000]);

    owner.remove();
    let late = writer.clone();
    let written = std::thread::spawn(move || late.set(1001)).join();

    assert!(written.is_ok(), "a write off the UI thread to a feed that is gone is dropped");
    assert_eq!(ui.run(), 0, "and nothing is sent to the UI thread for it");
    assert_eq!(*seen.borrow(), [1000]);
}
