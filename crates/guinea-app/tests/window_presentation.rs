use guinea_app::{
    app::{GuineaHost, roots::Root, windows::set_presentation},
    feature::ScopeContext,
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn presentation_is_window_local_distinct_and_independent_of_sleep() {
    let host = GuineaHost::new();
    let a = Root::under(&host.context());
    let b = Root::under(&host.context());
    let page = a.scope().child();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let copy = seen.clone();
    ScopeContext::new(page).on_window_presented(move |value| copy.borrow_mut().push(value));
    assert_eq!(&*seen.borrow(), &[true]);
    set_presentation(b.scope(), false, false);
    assert_eq!(&*seen.borrow(), &[true]);
    page.sleep();
    set_presentation(a.scope(), true, true);
    set_presentation(a.scope(), false, true);
    set_presentation(a.scope(), false, false);
    assert_eq!(&*seen.borrow(), &[true, false]);
    set_presentation(a.scope(), true, false);
    assert_eq!(&*seen.borrow(), &[true, false, true]);
    page.wake();
    assert_eq!(&*seen.borrow(), &[true, false, true]);
    page.remove();
    set_presentation(a.scope(), false, false);
    assert_eq!(&*seen.borrow(), &[true, false, true]);
}
#[test]
fn late_subscription_receives_current_state_even_when_asleep() {
    let host = GuineaHost::new();
    let root = Root::under(&host.context());
    set_presentation(root.scope(), false, false);
    let page = root.scope().child();
    page.sleep();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let copy = seen.clone();
    ScopeContext::new(page).on_window_presented(move |v| copy.borrow_mut().push(v));
    assert_eq!(&*seen.borrow(), &[false]);
}
#[test]
fn reentrant_changes_supersede_obsolete_notifications() {
    let host = GuineaHost::new();
    let root = Root::under(&host.context());
    let scope = root.scope();
    let cx = ScopeContext::new(scope.child());
    cx.on_window_presented(move |value| {
        if !value {
            set_presentation(scope, true, false)
        }
    });
    let seen = Rc::new(RefCell::new(Vec::new()));
    let copy = seen.clone();
    cx.on_window_presented(move |v| copy.borrow_mut().push(v));
    set_presentation(scope, false, false);
    assert_eq!(&*seen.borrow(), &[true]);
}
#[test]
#[should_panic(expected = "window presentation requires a window root")]
fn app_scope_is_not_an_arbitrary_window() {
    let host = GuineaHost::new();
    ScopeContext::new(host.context().__scope()).on_window_presented(|_| {});
}
