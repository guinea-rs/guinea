use std::cell::{Cell, RefCell};
use std::rc::Rc;

use guinea_core::guard::{Ask, Decision, Verdict};
use guinea_core::scope::ScopeTree;
use guinea_router::navigator::{Level, Moved, Navigator, Routable};

#[derive(Clone, Debug, PartialEq)]
enum Top {
    Processes(Processes),
    Services,
    Logs,
}

#[derive(Clone, Debug, PartialEq)]
enum Processes {
    List,
    Settings,
}

impl Routable for Top {
    fn name(&self) -> &'static str {
        match self {
            Top::Processes(_) => "Processes",
            Top::Services => "Services",
            Top::Logs => "Logs",
        }
    }

    fn stays(&self, to: &Self) -> bool {
        matches!((self, to), (Top::Processes(_), Top::Processes(_))) || self == to
    }

    fn levels() -> Vec<Level<Self>> {
        Level::nested(
            |top| match top {
                Top::Processes(inner) => Some(inner),
                _ => None,
            },
            Top::Processes,
        )
    }
}

impl Routable for Processes {
    fn name(&self) -> &'static str {
        match self {
            Processes::List => "List",
            Processes::Settings => "Settings",
        }
    }
}

fn opened() -> (ScopeTree, Navigator<Top>) {
    let tree = ScopeTree::new();
    let navigator = Navigator::open(tree.scope(), Top::Processes(Processes::List));
    (tree, navigator)
}

#[test]
fn a_navigator_is_found_below_where_it_was_opened_at_its_own_level_and_a_nested_one() {
    let (tree, _navigator) = opened();
    let below = tree.scope().child().child();

    let top = Navigator::<Top>::find(below).map(|found| found.current());
    let nested = Navigator::<Processes>::find(below).map(|found| found.current());

    assert_eq!(top, Some(Some(Top::Processes(Processes::List))));
    assert_eq!(nested, Some(Some(Processes::List)));
}

#[test]
fn going_somewhere_moves_it_and_back_and_forward_step_through_where_it_was() {
    let (_tree, navigator) = opened();

    assert!(matches!(navigator.to(Top::Services), Moved::Done));
    assert_eq!(navigator.current(), Some(Top::Services));

    assert!(navigator.back());
    assert_eq!(navigator.current(), Some(Top::Processes(Processes::List)));
    assert!(navigator.forward());
    assert_eq!(navigator.current(), Some(Top::Services));
    assert!(!navigator.forward(), "nothing further ahead");
}

#[test]
fn a_nested_level_goes_somewhere_within_its_own_variant() {
    let (tree, navigator) = opened();
    let nested = Navigator::<Processes>::find(tree.scope()).unwrap();

    nested.to(Processes::Settings);

    assert_eq!(navigator.current(), Some(Top::Processes(Processes::Settings)));
    navigator.to(Top::Services);
    assert_eq!(nested.current(), None, "outside its variant there is nothing at its level");
}

#[test]
fn only_an_outlet_whose_level_changes_has_its_child_asked_whether_it_may_go() {
    let (tree, navigator) = opened();
    let asked = Rc::new(RefCell::new(Vec::new()));

    let shell = tree.scope().child();
    let area = shell.child();
    let list = area.child();
    for (scope, name) in [(area, "area"), (list, "list")] {
        let asked = asked.clone();
        scope.on_leave(move || {
            asked.borrow_mut().push(name);
            Verdict::Allow
        });
    }
    navigator.rendered(shell, area, &Top::Processes(Processes::List));
    Navigator::<Processes>::find(area).unwrap().rendered(area, list, &Processes::List);

    navigator.to(Top::Processes(Processes::Settings));
    assert_eq!(*asked.borrow(), ["list"], "the area stays: only the page under it goes");

    asked.borrow_mut().clear();
    navigator.rendered(shell, area, &Top::Processes(Processes::Settings));
    navigator.to(Top::Services);
    assert_eq!(*asked.borrow(), ["list", "area"], "the whole area goes, innermost first");
}

#[test]
fn a_question_parks_the_navigation_until_it_is_answered_and_a_later_one_drops_it() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    page.on_leave(|| Verdict::ask(Ask::new("Discard?", "Discard", "Stay")));
    navigator.rendered(shell, page, &Top::Processes(Processes::List));

    assert!(matches!(navigator.to(Top::Services), Moved::Deferred));
    assert_eq!(navigator.pending().map(|ask| ask.text), Some("Discard?".into()));
    assert_eq!(navigator.current(), Some(Top::Processes(Processes::List)));

    navigator.answer(true);
    assert_eq!(navigator.current(), Some(Top::Services));
    assert_eq!(navigator.pending(), None);
}

#[test]
fn a_later_navigation_drops_the_question_an_earlier_one_was_waiting_on() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    let decision = guinea_core::guard::Decision::new();
    let asking = decision.clone();
    let asked = Cell::new(false);
    page.on_leave(move || match asked.replace(true) {
        false => Verdict::Ask(Ask::new("Discard?", "Discard", "Stay"), asking.clone()),
        true => Verdict::Block,
    });
    navigator.rendered(shell, page, &Top::Processes(Processes::List));

    assert!(matches!(navigator.to(Top::Services), Moved::Deferred));
    assert!(matches!(navigator.to(Top::Logs), Moved::Blocked));
    assert_eq!(navigator.pending(), None);

    decision.allow();
    assert_eq!(
        navigator.current(),
        Some(Top::Processes(Processes::List)),
        "the answer belonged to a navigation a later one superseded"
    );
}

#[test]
fn a_destination_that_fails_to_mount_puts_it_back_and_says_why() {
    let (_tree, navigator) = opened();

    navigator.to(Top::Services);
    navigator.failed(anyhow::anyhow!("no connection"));

    assert_eq!(navigator.current(), Some(Top::Processes(Processes::List)));
    assert_eq!(navigator.failure(), Some("no connection".to_string()));
    assert!(!navigator.back(), "the place it never reached is not in history");
}

#[test]
fn a_watcher_hears_a_question_come_and_go() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    page.on_leave(|| Verdict::ask(Ask::new("Discard?", "Discard", "Stay")));
    navigator.rendered(shell, page, &Top::Processes(Processes::List));

    let heard = Rc::new(Cell::new(0));
    let counting = heard.clone();
    let _watching = navigator.watch(move || counting.set(counting.get() + 1));

    navigator.to(Top::Services);
    assert_eq!(heard.get(), 1, "the question came");

    navigator.answer(false);
    assert_eq!(heard.get(), 2, "the question went, and it stayed");
    assert_eq!(navigator.current(), Some(Top::Processes(Processes::List)));
}

#[test]
fn an_outlet_is_told_what_it_drew_last() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let area = shell.child();
    let list = area.child();

    navigator.rendered(shell, area, &Top::Processes(Processes::List));
    Navigator::<Processes>::find(area).unwrap().rendered(area, list, &Processes::List);

    assert_eq!(navigator.drawn(shell), Some(area));
    assert_eq!(Navigator::<Processes>::find(area).unwrap().drawn(area), Some(list));
    assert_eq!(navigator.drawn(area), None, "the area's outlet is at another level");
}

#[test]
fn a_watcher_hears_every_move_until_it_lets_go() {
    let (_tree, navigator) = opened();
    let heard = Rc::new(Cell::new(0));

    let counting = heard.clone();
    let watching = navigator.watch(move || counting.set(counting.get() + 1));
    navigator.to(Top::Services);
    navigator.back();
    drop(watching);
    navigator.forward();

    assert_eq!(heard.get(), 2);
}

#[test]
fn a_handler_goes_where_it_was_made_for_each_time_it_is_called() {
    let (_tree, navigator) = opened();
    let go = navigator.to_handler(Top::Services);

    go();
    assert_eq!(navigator.current(), Some(Top::Services));

    navigator.back();
    go();
    assert_eq!(navigator.current(), Some(Top::Services));
    assert!(navigator.can_go_back());
}

#[test]
fn a_guard_may_settle_its_own_question_later_and_the_navigation_waits_for_it() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    let decision = Decision::new();
    let asking = decision.clone();
    page.on_leave(move || Verdict::Ask(Ask::new("Discard?", "Discard", "Stay"), asking.clone()));
    navigator.rendered(shell, page, &Top::Processes(Processes::List));

    assert!(matches!(navigator.to(Top::Services), Moved::Deferred));
    assert_eq!(navigator.pending().map(|ask| ask.text), Some("Discard?".into()));
    assert_eq!(navigator.current(), Some(Top::Processes(Processes::List)));

    decision.allow();

    assert_eq!(navigator.current(), Some(Top::Services));
    assert_eq!(navigator.pending(), None);
}

#[test]
fn a_leave_guard_that_refuses_leaves_everything_where_it_was() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    page.on_leave(|| Verdict::Block);
    navigator.rendered(shell, page, &Top::Services);
    navigator.to(Top::Services);

    assert!(matches!(navigator.to(Top::Logs), Moved::Blocked));
    assert_eq!(navigator.current(), Some(Top::Services));
    assert!(page.is_alive());

    assert!(
        matches!(navigator.to(Top::Services), Moved::Done),
        "staying where it is does not ask what would not be left"
    );
}

#[test]
fn a_second_navigation_asks_again_for_its_own_destination() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    page.on_leave(|| Verdict::ask(Ask::new("Discard?", "Discard", "Stay")));
    navigator.rendered(shell, page, &Top::Processes(Processes::List));

    assert!(matches!(navigator.to(Top::Services), Moved::Deferred));
    assert!(matches!(navigator.to(Top::Logs), Moved::Deferred));
    navigator.answer(true);

    assert_eq!(navigator.current(), Some(Top::Logs));
}

#[test]
fn a_guard_that_answered_before_it_returned_lets_the_navigation_through() {
    let (tree, navigator) = opened();
    let shell = tree.scope().child();
    let page = shell.child();
    page.on_leave(|| {
        let decision = Decision::new();
        decision.allow();
        Verdict::Ask(Ask::new("Discard?", "Discard", "Stay"), decision)
    });
    navigator.rendered(shell, page, &Top::Processes(Processes::List));

    assert!(matches!(navigator.to(Top::Services), Moved::Done));
    assert_eq!(navigator.current(), Some(Top::Services));
    assert_eq!(navigator.pending(), None);
}

/// A page at `at` that asks before it is left while `asking` says so.
fn minding(tree: &ScopeTree, navigator: &Navigator<Top>, at: Top) -> Rc<Cell<bool>> {
    let shell = tree.scope().child();
    let page = shell.child();
    let asking = Rc::new(Cell::new(true));
    let minds = asking.clone();
    page.on_leave(move || match minds.get() {
        true => Verdict::ask(Ask::new("Discard?", "Discard", "Stay")),
        false => Verdict::Allow,
    });
    navigator.rendered(shell, page, &at);
    asking
}

#[test]
fn a_back_that_supersedes_a_waiting_back_is_one_step_from_where_it_stands() {
    let (tree, navigator) = opened();
    navigator.to(Top::Services);
    navigator.to(Top::Logs);
    let asking = minding(&tree, &navigator, Top::Logs);

    navigator.back();
    navigator.back();
    navigator.answer(true);
    assert_eq!(navigator.current(), Some(Top::Services));

    asking.set(false);
    navigator.back();
    assert_eq!(navigator.current(), Some(Top::Processes(Processes::List)));
}

#[test]
fn a_forward_that_supersedes_a_waiting_forward_is_one_step_from_where_it_stands() {
    let (tree, navigator) = opened();
    navigator.to(Top::Services);
    navigator.to(Top::Logs);
    navigator.back();
    navigator.back();
    let asking = minding(&tree, &navigator, Top::Processes(Processes::List));

    navigator.forward();
    navigator.forward();
    navigator.answer(true);
    assert_eq!(navigator.current(), Some(Top::Services));

    asking.set(false);
    navigator.forward();
    assert_eq!(navigator.current(), Some(Top::Logs));
}
