//! What reading says when the segment it reads from is already gone.

use std::rc::Rc;

use guinea_core::scope::ScopeTree;
use guinea_macros::reducer;
use guinea_router::headless::Headless;
use guinea_router::router::SegmentProps;

#[derive(Clone, Default, Debug)]
struct Count;

#[reducer]
fn count(_this: &mut Count, _: ()) {}

#[test]
fn reading_from_a_torn_down_segment_says_it_was_torn_down() {
    let tree = ScopeTree::new();
    let page = tree.child();
    page.section().claim::<Count>(None);
    page.remove();
    let props = SegmentProps::<Headless> {
        chain: &[],
        scopes: Rc::new(vec![page.into()]),
        cursor: 0,
    };

    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        props.binding::<Count>();
    }));

    let said = read
        .err()
        .and_then(|panic| panic.downcast::<String>().ok())
        .map(|said| *said)
        .unwrap_or_default();
    assert!(said.contains("torn down"), "said: {said}");
}
