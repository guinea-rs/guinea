//! The window chrome owns a guard dialog, not the selected route.
use guinea_router::navigator::{Asking, Watching};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn watch_questions(navigator: &Rc<dyn Asking>, notify: impl Fn() + 'static) -> Watching {
    let previous = RefCell::new(navigator.pending());
    let source = Rc::downgrade(navigator);
    navigator.watch(Box::new(move || {
        let Some(source) = source.upgrade() else {
            return;
        };
        let current = source.pending();
        if *previous.borrow() == current {
            return;
        }
        *previous.borrow_mut() = current;
        notify();
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_app::app::{GuineaHost, roots::Root};
    use guinea_core::guard::{Ask, Verdict};
    use guinea_router::navigator::{Moved, Navigator};
    use std::cell::Cell;

    #[derive(Clone, Debug, PartialEq, guinea_macros::Routable)]
    enum Route {
        First,
        Second,
    }

    #[test]
    fn route_changes_leave_chrome_alone_but_questions_open_and_close() {
        let host = GuineaHost::new();
        let root = Root::under(&host.context());
        let nav = Navigator::open(root.scope(), Route::First);
        let asking: Rc<dyn Asking> = Rc::new(nav);
        let changes = Rc::new(Cell::new(0));
        let counted = changes.clone();
        let watch = watch_questions(&asking, move || counted.set(counted.get() + 1));
        assert_eq!(nav.to(Route::Second), Moved::Done);
        assert_eq!(nav.to(Route::First), Moved::Done);
        assert_eq!(changes.get(), 0);

        let child = root.scope().child();
        nav.rendered(root.scope(), child, &Route::First);
        child.on_leave(|| Verdict::ask(Ask::new("Leave?", "Leave", "Stay")));
        assert_eq!(nav.to(Route::Second), Moved::Deferred);
        assert_eq!(changes.get(), 1);
        nav.answer(false);
        assert_eq!(nav.current(), Some(Route::First));
        assert_eq!(changes.get(), 2);
        assert_eq!(nav.to(Route::Second), Moved::Deferred);
        assert_eq!(changes.get(), 3);
        nav.answer(true);
        assert_eq!(nav.current(), Some(Route::Second));
        assert_eq!(changes.get(), 4);
        drop(watch);
        let _ = nav.to(Route::First);
        assert_eq!(changes.get(), 4);
    }
}
