//! Where a reducer's change becomes a redraw - after the turn that made it.
//!
//! [`Scope::push`](crate::scope::Scope::push) applies the update and marks the
//! cell. The listeners run when the current *turn* ends: a turn is one delivery
//! of messages to an actor, or, when nothing opened one, the push itself.
//!
//! The unit is deliberately the turn and not the frame. Tying it to a frame
//! makes the delay a property of the backend - the next tick under an
//! immediate-mode toolkit, whenever the loop wakes under a retained one - and
//! a page author cannot say when their change lands. Tying it to the turn puts
//! the drain microseconds later, in the same place on every backend.
//!
//! Two things follow. The actor loop and the drawing loop stop sharing a
//! stack: the listeners run after `handle` returned, so navigating or dropping
//! a scope from one cannot pull the ground from under an actor that is still
//! running. And a burst of pushes inside one turn costs one notification per
//! cell rather than one per push, because marks are deduplicated.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::time::Instant;

use crate::scope::Scope;
use crate::trace::{self, Cause};

/// One applied update, on its way to whoever is watching.
struct Change {
    scope: Scope,
    cell: TypeId,
    update: Option<Box<dyn Any>>,
    /// What was happening when the update was applied. The listeners run a
    /// drain later, and what they set off - a redraw, most often - is still
    /// its doing.
    cause: Option<Cause>,
}

/// A feed that took a value, on its way to its followers.
struct Fed {
    scope: Scope,
    /// What its scope holds it under.
    key: usize,
    /// Hands the feed's value now to each follower that has not seen it.
    deliver: fn(Scope, usize),
}

thread_local! {
    static MARKED: RefCell<Vec<Change>> = const { RefCell::new(Vec::new()) };
    static FED: RefCell<Vec<Fed>> = const { RefCell::new(Vec::new()) };
    static OPENED: Cell<Option<Instant>> = const { Cell::new(None) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static DRAINING: Cell<bool> = const { Cell::new(false) };
}

struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let depth = DEPTH.with(|depth| {
            let now = depth.get().saturating_sub(1);
            depth.set(now);
            now
        });

        if depth == 0 && !std::thread::panicking() {
            drain();
        }
    }
}

/// Runs `f` as one turn: everything it marks is notified once, after it
/// returns, and nested turns fold into the outermost one.
///
/// For whoever delivers work to the application - an actor's queue, an event
/// from the toolkit. A push made outside any turn notifies at once, since
/// there is no frame beneath it to protect.
pub fn turn<R>(f: impl FnOnce() -> R) -> R {
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = Guard;
    f()
}

pub(crate) fn mark(scope: Scope, cell: TypeId, update: Option<Box<dyn Any>>) {
    let was_pending = pending();
    MARKED.with(|marked| {
        let mut marked = marked.borrow_mut();

        let seen = marked.iter().any(|c| c.cell == cell && c.scope == scope);

        // An observed update is kept whole - a rename and a fresh list are
        // different things to whoever is watching. A bare mark says only that
        // the cell moved, so one of those is as good as ten.
        if update.is_some() || !seen {
            marked.push(Change {
                scope,
                cell,
                update,
                cause: trace::current(),
            });
        }
    });

    marked(was_pending);
}

/// Marks the feed `scope` holds under `key` as moved: its followers are
/// handed the value it has when the turn ends, with the state the turn
/// changed.
pub(crate) fn feed(scope: Scope, key: usize, deliver: fn(Scope, usize)) {
    let was_pending = pending();
    FED.with(|fed| {
        let mut fed = fed.borrow_mut();
        if !fed.iter().any(|fed| fed.key == key) {
            fed.push(Fed { scope, key, deliver });
        }
    });

    marked(was_pending);
}

fn marked(was_pending: bool) {
    if !was_pending {
        OPENED.with(|opened| opened.set(Some(Instant::now())));
    }

    let free = DEPTH.with(|depth| depth.get()) == 0 && !DRAINING.with(|d| d.get());
    if free {
        drain();
    }
}

/// How many times state may settle before we stop chasing it.
///
/// An observer that pushes into a reducer another observer watches is a chain,
/// and a legitimate one; a chain that never ends is a bug, and this is where it
/// surfaces as a message rather than a hang.
const SETTLE_ROUNDS: usize = 16;

/// Runs the listeners of every cell marked since the last call, in the order
/// the cells were first marked.
///
/// One drain at a time. A listener that marks a cell - directly, or through
/// an actor whose turn ends inside it - adds a round to this drain instead of
/// starting one of its own, and every round counts against the same
/// [`SETTLE_ROUNDS`]: a cycle ends in a warning, not in a stack that never
/// stops growing.
pub(crate) fn drain() {
    if DRAINING.with(Cell::get) || !pending() {
        return;
    }

    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            DRAINING.with(|draining| draining.set(false));
        }
    }
    DRAINING.with(|draining| draining.set(true));
    let _running = Running;

    let opened = OPENED.with(|opened| opened.take());

    let mut updates = 0usize;
    let mut gone = 0usize;
    let mut rounds = 0usize;
    let mut cells = 0usize;
    let mut listeners = 0usize;
    let mut feeds = 0usize;

    'drained: while pending() {
        let mut touched: Vec<(Scope, TypeId, Option<Cause>)> = Vec::new();

        // State first, and until it stops moving: an observer turning someone
        // else's update into its own is how one piece of state follows another,
        // and all of it has to settle before anything is told to redraw. A
        // feed's follower is one of those.
        while pending() {
            rounds += 1;
            if rounds > SETTLE_ROUNDS {
                tracing::warn!(
                    rounds = SETTLE_ROUNDS,
                    "state did not settle - an observer or a listener is feeding itself"
                );
                MARKED.with(|marked| marked.borrow_mut().clear());
                FED.with(|fed| fed.borrow_mut().clear());
                break 'drained;
            }

            let batch = MARKED.with(|marked| std::mem::take(&mut *marked.borrow_mut()));

            for change in batch {
                if !change.scope.is_alive() {
                    gone += 1;
                    continue;
                }

                let seen = touched
                    .iter()
                    .any(|&(scope, cell, _)| scope == change.scope && cell == change.cell);
                if !seen {
                    touched.push((change.scope, change.cell, change.cause));
                }

                let Some(update) = change.update else { continue };
                let _under = trace::resume(change.cause);
                for observer in change.scope.observers_of(change.cell) {
                    if observer.is_gone() || !change.scope.is_alive() {
                        continue;
                    }

                    updates += 1;
                    (observer.run)(&*update);
                }
            }

            let fed = FED.with(|fed| std::mem::take(&mut *fed.borrow_mut()));
            for fed in fed {
                if !fed.scope.is_alive() {
                    gone += 1;
                    continue;
                }

                feeds += 1;
                (fed.deliver)(fed.scope, fed.key);
            }
        }

        cells += touched.len();

        for (scope, cell, cause) in touched {
            if !scope.is_alive() {
                gone += 1;
                continue;
            }

            let _under = trace::resume(cause);
            for listener in scope.listeners_of(cell) {
                if listener.is_gone() || !scope.is_alive() {
                    continue;
                }

                listeners += 1;
                (listener.run)();
            }
        }
    }

    tracing::debug!(
        cells,
        listeners,
        updates,
        feeds,
        rounds,
        gone,
        waited_us = opened.map(|at| at.elapsed().as_micros()).unwrap_or(0),
        "drained"
    );
}

/// Whether any cell or feed is waiting to be drained.
pub(crate) fn pending() -> bool {
    MARKED.with(|marked| !marked.borrow().is_empty()) || FED.with(|fed| !fed.borrow().is_empty())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell as StdCell;
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::scope::ScopeTree;

    #[derive(Clone, Default, Debug)]
    struct Count(u32);

    #[guinea_macros::reducer]
    fn count(this: &mut Count, to: u32) {
        this.0 = to;
    }

    /// A second cell, for watching one piece of state follow another.
    #[derive(Clone, Default, Debug)]
    struct Mirror(u32);

    #[guinea_macros::reducer]
    fn mirror(this: &mut Mirror, to: u32) {
        this.0 = to;
    }

    fn watched() -> (ScopeTree, Rc<StdCell<u32>>, crate::scope::Subscription) {
        let scope = ScopeTree::new();
        let runs = Rc::new(StdCell::new(0));
        let sub = scope.subscribe::<Count>({
            let runs = runs.clone();
            move || runs.set(runs.get() + 1)
        });
        (scope, runs, sub)
    }

    #[test]
    fn a_push_outside_a_turn_notifies_at_once() {
        let (scope, runs, _sub) = watched();

        scope.push::<Count>(7);

        assert_eq!(scope.state::<Count>().borrow().0, 7);
        assert_eq!(runs.get(), 1, "nothing was underneath, so nothing had to wait");
    }

    #[test]
    fn a_push_inside_a_turn_waits_for_it_to_end() {
        let (scope, runs, _sub) = watched();

        super::turn(|| {
            scope.push::<Count>(7);
            assert_eq!(
                scope.state::<Count>().borrow().0,
                7,
                "state is current inside the turn"
            );
            assert_eq!(runs.get(), 0, "the listener did not run on the pusher's stack");
        });

        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn a_burst_inside_one_turn_costs_one_notification() {
        let (scope, runs, _sub) = watched();

        super::turn(|| {
            for n in 0..5 {
                scope.push::<Count>(n);
            }
        });

        assert_eq!(runs.get(), 1);
        assert_eq!(scope.state::<Count>().borrow().0, 4);
    }

    #[test]
    fn nested_turns_notify_once_at_the_outermost_end() {
        let (scope, runs, _sub) = watched();

        super::turn(|| {
            scope.push::<Count>(1);
            super::turn(|| {
                scope.push::<Count>(2);
            });
            assert_eq!(runs.get(), 0, "the inner turn did not drain under the outer one");
        });

        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn a_scope_removed_before_the_turn_ends_notifies_nobody() {
        let (scope, runs, _sub) = watched();

        super::turn(|| {
            scope.push::<Count>(1);
            scope.remove();
        });

        assert_eq!(runs.get(), 0);
    }

    #[test]
    fn an_observer_is_handed_the_update_itself() {
        let scope = ScopeTree::new();
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _sub = scope.observe::<Count>({
            let seen = seen.clone();
            move |update| seen.borrow_mut().push(*update)
        });

        super::turn(|| {
            scope.push::<Count>(1);
            scope.push::<Count>(2);
        });

        assert_eq!(
            *seen.borrow(),
            vec![1, 2],
            "every update reaches an observer - a rename is not a fresh list"
        );
    }

    #[test]
    fn observers_run_before_listeners_and_state_settles_first() {
        let scope = ScopeTree::new();
        let order = Rc::new(RefCell::new(Vec::new()));

        let _observer = scope.observe::<Count>({
            let order = order.clone();
            move |_| order.borrow_mut().push("observe")
        });
        let _listener = scope.subscribe::<Count>({
            let order = order.clone();
            move || order.borrow_mut().push("listen")
        });

        super::turn(|| scope.push::<Count>(1));

        assert_eq!(*order.borrow(), vec!["observe", "listen"]);
    }

    #[test]
    fn an_observer_feeding_another_cell_settles_before_anything_draws() {
        let scope = ScopeTree::new();
        let order = Rc::new(RefCell::new(Vec::new()));

        let held = scope.scope();
        let _follows = scope.observe::<Count>(move |update| held.push::<Mirror>(*update * 10));
        let _mirror_listener = scope.subscribe::<Mirror>({
            let order = order.clone();
            move || order.borrow_mut().push(held.state::<Mirror>().borrow().0)
        });

        super::turn(|| scope.push::<Count>(4));

        assert_eq!(
            *order.borrow(),
            vec![40],
            "the follower had already caught up by the time its listener ran"
        );
    }

    #[test]
    fn a_push_from_a_listener_is_another_round_of_the_same_drain() {
        let scope = ScopeTree::new();
        let runs = Rc::new(StdCell::new(0));

        let held = scope.scope();
        let _sub = scope.subscribe::<Count>({
            let runs = runs.clone();
            move || {
                runs.set(runs.get() + 1);
                if runs.get() < 3 {
                    held.push::<Count>(99);
                }
            }
        });

        super::turn(|| scope.push::<Count>(1));

        assert_eq!(runs.get(), 3, "nothing is left waiting for a drain nobody starts");
        assert!(!super::pending());
    }

    #[test]
    fn a_listener_whose_scope_an_earlier_listener_removed_is_not_called() {
        let tree = ScopeTree::new();
        let page = tree.child();
        let late = Rc::new(StdCell::new(0));

        page.hold_watching(page.subscribe::<Count>(move || page.remove()));
        let counted = late.clone();
        page.hold_watching(page.subscribe::<Count>(move || counted.set(counted.get() + 1)));

        page.push::<Count>(1);

        assert!(!page.is_alive());
        assert_eq!(late.get(), 0, "the second listener ran after its scope was removed");
    }

    #[test]
    fn a_page_listening_to_app_state_does_not_run_after_the_app_navigated_it_away() {
        let tree = ScopeTree::new();
        let app = tree.scope();
        let page = tree.child();
        let late = Rc::new(StdCell::new(0));

        app.hold_watching(app.subscribe::<Count>(move || page.remove()));
        let counted = late.clone();
        page.hold_watching(app.subscribe::<Count>(move || counted.set(counted.get() + 1)));

        app.push::<Count>(1);

        assert_eq!(late.get(), 0, "a removed page's listener ran");
    }

    #[test]
    fn an_observer_whose_scope_an_earlier_observer_removed_is_not_called() {
        let tree = ScopeTree::new();
        let page = tree.child();
        let late = Rc::new(StdCell::new(0));

        page.hold_watching(page.observe::<Count>(move |_| page.remove()));
        let counted = late.clone();
        page.hold_watching(page.observe::<Count>(move |_| counted.set(counted.get() + 1)));

        page.push::<Count>(1);

        assert!(!page.is_alive());
        assert_eq!(late.get(), 0, "the second observer ran after its scope was removed");
    }

    #[test]
    fn a_listener_dropped_by_an_earlier_listener_is_not_called() {
        let tree = ScopeTree::new();
        let late = Rc::new(StdCell::new(0));
        let second = Rc::new(RefCell::new(None));

        let dropping = second.clone();
        let _first = tree.subscribe::<Count>(move || drop(dropping.borrow_mut().take()));
        let counted = late.clone();
        *second.borrow_mut() = Some(tree.subscribe::<Count>(move || counted.set(counted.get() + 1)));

        tree.push::<Count>(1);

        assert!(second.borrow().is_none());
        assert_eq!(late.get(), 0, "the listener ran after its Subscription was dropped");
    }

    #[test]
    fn a_listener_whose_cell_was_seeded_over_is_not_called() {
        let tree = ScopeTree::new();
        let late = Rc::new(StdCell::new(0));

        let held = tree.scope();
        let _first = tree.subscribe::<Count>(move || held.seed(Count(0)));
        let counted = late.clone();
        let _second = tree.subscribe::<Count>(move || counted.set(counted.get() + 1));

        tree.push::<Count>(1);

        assert_eq!(late.get(), 0, "seed drops a cell's listeners, yet this one ran");
    }

    /// A listener that sends to an actor, whose handler pushes to the cell the
    /// listener watches: the turn the actor opens ends inside the listener.
    fn feeding_itself(through_a_turn: bool) -> usize {
        let scope = ScopeTree::new();
        let runs = Rc::new(StdCell::new(0));

        let held = scope.scope();
        let _sub = scope.subscribe::<Count>({
            let runs = runs.clone();
            move || {
                runs.set(runs.get() + 1);
                let n = runs.get() as u32;
                if through_a_turn {
                    super::turn(|| held.push::<Count>(n));
                } else {
                    held.push::<Count>(n);
                }
            }
        });

        super::turn(|| scope.push::<Count>(0));
        assert!(!super::pending());
        runs.get()
    }

    #[test]
    fn a_listener_feeding_itself_stops_at_the_limit() {
        assert_eq!(feeding_itself(false), super::SETTLE_ROUNDS);
    }

    #[test]
    fn a_listener_feeding_itself_through_a_turn_stops_at_the_same_limit() {
        assert_eq!(
            feeding_itself(true),
            super::SETTLE_ROUNDS,
            "the turn ending inside the listener did not start a drain of its own"
        );
    }
}
