use super::*;
use std::marker::PhantomData;

struct Probe<T>(PhantomData<T>);

#[allow(dead_code)]
trait NotCopy {
    fn is_copy(&self) -> bool {
        false
    }
}

impl<T> NotCopy for Probe<T> {}

impl<T: Copy> Probe<T> {
    fn is_copy(&self) -> bool {
        true
    }
}

#[allow(dead_code)]
trait Stays {
    fn is_send(&self) -> bool {
        false
    }
}

impl<T> Stays for Probe<T> {}

#[allow(dead_code)]
impl<T: Send> Probe<T> {
    fn is_send(&self) -> bool {
        true
    }
}

#[derive(Clone, Default, Debug)]
struct Count(u32);

#[guinea_macros::reducer]
fn count(this: &mut Count, by: u32) {
    this.0 += by;
}

#[test]
fn a_push_is_handed_on_by_copy_whatever_it_pushes_into() {
    assert!(Probe::<Push<Count>>(PhantomData).is_copy());
}

#[test]
fn a_dispatcher_is_handed_on_by_copy() {
    assert!(Probe::<Dispatch>(PhantomData).is_copy());
}

#[test]
fn a_push_and_a_dispatcher_stay_on_the_thread_their_scope_lives_on() {
    assert_eq!(
        (
            Probe::<Push<Count>>(PhantomData).is_send(),
            Probe::<Dispatch>(PhantomData).is_send()
        ),
        (false, false)
    );
}

#[test]
fn what_a_feature_keeps_of_its_reducer_is_handed_on_by_copy() {
    assert!(Probe::<Bound<Count>>(PhantomData).is_copy());
}
