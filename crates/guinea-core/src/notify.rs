//! Where a reducer's change becomes a redraw - after the turn that made it.
//!
//! [`Scope::push`](crate::scope::Scope::push) applies the update and marks the
//! cell. When the current *turn* ends - one delivery of messages to an actor,
//! or, when nothing opened one, the push itself - the cell's observers are
//! handed the update until state settles, and then each tree's
//! [`Draws`](crate::draws::Draws) is handed, in one call, every reader of a
//! cell that moved.
//!
//! The unit is deliberately the turn and not the frame. Tying it to a frame
//! makes the delay a property of the backend - the next tick under an
//! immediate-mode toolkit, whenever the loop wakes under a retained one - and
//! a page author cannot say when their change lands. Tying it to the turn puts
//! the drain microseconds later, in the same place on every backend.
//!
//! Two things follow. The actor loop and the drawing loop stop sharing a
//! stack: the redraw comes after `handle` returned, so navigating or dropping
//! a scope from one cannot pull the ground from under an actor that is still
//! running. And a burst of pushes inside one turn costs one notification per
//! cell rather than one per push, because marks are deduplicated.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::draws::Read;
use crate::scope::{Scope, Watcher};
use crate::trace::{self, Cause};

/// One applied update, on its way to whoever is watching.
struct Change {
    scope: Scope,
    cell: TypeId,
    update: Option<Box<dyn Any>>,
    /// What was happening when the update was applied. Its observers and
    /// readers run a drain later, and what they set off is still its doing.
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
/// An actor opens one for each delivery to it, and nothing else in guinea
/// does: an action answered by a closure, a toolkit's event or a timer runs
/// outside any turn, and each push it makes notifies at once, since there is
/// no actor beneath it to protect.
///
/// ```
/// # use std::cell::Cell;
/// # use std::rc::Rc;
/// # use guinea_core::draws::{Draws, Readers};
/// # use guinea_core::notify;
/// # use guinea_core::scope::ScopeTree;
/// #[derive(Clone, Default, Debug)]
/// struct Count(u32);
///
/// #[guinea_macros::reducer]
/// fn count(this: &mut Count, to: u32) {
///     this.0 = to;
/// }
///
/// struct Counting(Rc<Cell<u32>>);
///
/// impl Draws for Counting {
///     type Reader = ();
///
///     fn redraw(&self, _: Readers<'_, ()>) {
///         self.0.set(self.0.get() + 1);
///     }
/// }
///
/// let tree = ScopeTree::new();
/// let redrawn = Rc::new(Cell::new(0));
/// tree.drawn_by(Counting(redrawn.clone()));
/// let _reading = tree.binding::<Count>().read_by(());
///
/// tree.push::<Count>(1);
/// tree.push::<Count>(2);
/// assert_eq!(redrawn.get(), 2);
///
/// notify::turn(|| {
///     tree.push::<Count>(3);
///     tree.push::<Count>(4);
/// });
/// assert_eq!(redrawn.get(), 3);
/// ```
pub fn turn<R>(f: impl FnOnce() -> R) -> R {
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = Guard;
    f()
}

pub(crate) fn mark(scope: Scope, cell: TypeId, update: Option<Box<dyn Any>>) {
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

    drain_outside_turn();
}

/// Marks the feed `scope` holds under `key` as moved: its followers are
/// handed the value it has when the turn ends, with the state the turn
/// changed.
pub(crate) fn feed(scope: Scope, key: usize, deliver: fn(Scope, usize)) {
    FED.with(|fed| {
        let mut fed = fed.borrow_mut();
        if !fed.iter().any(|fed| fed.key == key) {
            fed.push(Fed { scope, key, deliver });
        }
    });

    drain_outside_turn();
}

/// A mark made outside any turn has no frame beneath it to protect, and is
/// drained at once.
fn drain_outside_turn() {
    if DEPTH.with(Cell::get) == 0 {
        drain();
    }
}

/// How many times state may settle before we stop chasing it.
///
/// An observer that pushes into a reducer another observer watches is a chain,
/// and a legitimate one; a chain that never ends is a bug, and this is where it
/// surfaces as a message rather than a hang.
const SETTLE_ROUNDS: usize = 16;

/// Settles every cell marked since the last call, then redraws the readers of
/// each, in the order the cells were first marked.
///
/// One drain at a time. A redraw that marks a cell - directly, or through
/// an actor whose turn ends inside it - adds a round to this drain instead of
/// starting one of its own, and every round counts against the same
/// [`SETTLE_ROUNDS`]: a cycle ends in a warning, not in a stack that never
/// stops growing.
pub(crate) fn drain() {
    let Some(_draining) = Draining::start() else {
        return;
    };

    let mut rounds = 0;
    while pending() {
        let Some(touched) = settle(&mut rounds) else {
            return;
        };
        redraw(touched);
    }
}

/// The drain under way on this thread. There is one at a time.
struct Draining;

impl Draining {
    fn start() -> Option<Self> {
        match DRAINING.replace(true) {
            true => None,
            false => Some(Self),
        }
    }
}

impl Drop for Draining {
    fn drop(&mut self) {
        DRAINING.set(false);
    }
}

/// A cell that moved during a drain, and what moved it.
struct Touched {
    scope: Scope,
    cell: TypeId,
    cause: Option<Cause>,
}

/// Hands each update to its cell's observers and each fed value to its
/// followers, round after round, until nothing is marked.
///
/// An observer turning one update into another is how one piece of state
/// follows another, and a feed's follower is one of those: all of it settles
/// before anything is told to redraw. Comes back with the cells it touched,
/// each once, in the order first marked; `None` when it did not settle
/// within [`SETTLE_ROUNDS`].
fn settle(rounds: &mut usize) -> Option<Vec<Touched>> {
    let mut touched: Vec<Touched> = Vec::new();

    while pending() {
        *rounds += 1;
        if *rounds > SETTLE_ROUNDS {
            tracing::warn!(
                rounds = SETTLE_ROUNDS,
                "state did not settle - an observer or a redraw is feeding itself"
            );
            MARKED.with_borrow_mut(Vec::clear);
            FED.with_borrow_mut(Vec::clear);
            return None;
        }

        for Change { scope, cell, update, cause } in MARKED.take() {
            if !scope.is_alive() {
                continue;
            }

            if !touched.iter().any(|seen| seen.scope == scope && seen.cell == cell) {
                touched.push(Touched { scope, cell, cause });
            }

            let Some(update) = update else { continue };
            let _under = trace::resume(cause);
            each_present(scope, scope.observers_of(cell), |observe| observe(&*update));
        }

        for fed in FED.take() {
            if fed.scope.is_alive() {
                (fed.deliver)(fed.scope, fed.key);
            }
        }
    }

    Some(touched)
}

/// Hands each tree's [`Draws`](crate::draws::Draws) the readers of the
/// cells it touched, in one call.
fn redraw(touched: Vec<Touched>) {
    let mut reads: Vec<Read> = touched
        .into_iter()
        .flat_map(|Touched { scope, cell, cause }| {
            scope
                .readers_of(cell)
                .into_iter()
                .map(move |reader| Read { scope, reader, cause })
        })
        .collect();

    while let Some(first) = reads.first().map(|read| read.scope) {
        let (tree, rest) = reads.into_iter().partition(|read| read.scope.shares_tree(first));
        reads = rest;

        if let Some(drawing) = first.tree_drawing() {
            drawing.redraw(&tree);
        }
    }
}

/// Runs `run` on each watcher of `scope` that has not left, while `scope`
/// lives: any of them may remove the scope, or a watcher after it.
fn each_present<F: ?Sized>(scope: Scope, watchers: Vec<Rc<Watcher<F>>>, run: impl Fn(&F)) {
    for watcher in watchers {
        if !scope.is_alive() {
            return;
        }
        if !watcher.is_gone() {
            run(&watcher.run);
        }
    }
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

    use crate::scope::{Reducer, Scope, ScopeTree, Subscription};

    fn drawn() -> ScopeTree {
        let tree = ScopeTree::new();
        tree.drawn_by(crate::draws::Calls);
        tree
    }

    fn read<R: Reducer>(scope: &Scope, redraw: impl Fn() + 'static) -> Subscription {
        scope.binding::<R>().read_by(Box::new(redraw) as Box<dyn Fn()>)
    }

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
        let scope = drawn();
        let runs = Rc::new(StdCell::new(0));
        let sub = read::<Count>(&scope, {
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
            assert_eq!(runs.get(), 0, "the reader was not redrawn on the pusher's stack");
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
        let scope = drawn();
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
    fn observers_run_before_anything_is_redrawn() {
        let scope = drawn();
        let order = Rc::new(RefCell::new(Vec::new()));

        let _observer = scope.observe::<Count>({
            let order = order.clone();
            move |_| order.borrow_mut().push("observe")
        });
        let _reader = read::<Count>(&scope, {
            let order = order.clone();
            move || order.borrow_mut().push("redraw")
        });

        super::turn(|| scope.push::<Count>(1));

        assert_eq!(*order.borrow(), vec!["observe", "redraw"]);
    }

    #[test]
    fn an_observer_feeding_another_cell_settles_before_anything_draws() {
        let scope = drawn();
        let order = Rc::new(RefCell::new(Vec::new()));

        let held = scope.scope();
        let _follows = scope.observe::<Count>(move |update| held.push::<Mirror>(*update * 10));
        let _mirror_reader = read::<Mirror>(&scope, {
            let order = order.clone();
            move || order.borrow_mut().push(held.state::<Mirror>().borrow().0)
        });

        super::turn(|| scope.push::<Count>(4));

        assert_eq!(
            *order.borrow(),
            vec![40],
            "the follower had already caught up by the time its reader was redrawn"
        );
    }

    #[test]
    fn a_push_from_a_redraw_is_another_round_of_the_same_drain() {
        let scope = drawn();
        let runs = Rc::new(StdCell::new(0));

        let held = scope.scope();
        let _sub = read::<Count>(&scope, {
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
    fn a_reader_whose_scope_an_earlier_redraw_removed_is_not_redrawn() {
        let tree = drawn();
        let page = tree.child();
        let late = Rc::new(StdCell::new(0));

        page.hold_watching(read::<Count>(&page, move || page.remove()));
        let counted = late.clone();
        page.hold_watching(read::<Count>(&page, move || counted.set(counted.get() + 1)));

        page.push::<Count>(1);

        assert!(!page.is_alive());
        assert_eq!(late.get(), 0, "the second reader was redrawn after its scope was removed");
    }

    #[test]
    fn a_page_reading_app_state_is_not_redrawn_after_the_app_navigated_it_away() {
        let tree = drawn();
        let app = tree.scope();
        let page = tree.child();
        let late = Rc::new(StdCell::new(0));

        app.hold_watching(read::<Count>(&app, move || page.remove()));
        let counted = late.clone();
        page.hold_watching(read::<Count>(&app, move || counted.set(counted.get() + 1)));

        app.push::<Count>(1);

        assert_eq!(late.get(), 0, "a removed page's reader was redrawn");
    }

    #[test]
    fn an_observer_whose_scope_an_earlier_observer_removed_is_not_called() {
        let tree = drawn();
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
    fn a_reader_dropped_by_an_earlier_redraw_is_not_redrawn() {
        let tree = drawn();
        let late = Rc::new(StdCell::new(0));
        let second = Rc::new(RefCell::new(None));

        let dropping = second.clone();
        let _first = read::<Count>(&tree, move || drop(dropping.borrow_mut().take()));
        let counted = late.clone();
        *second.borrow_mut() = Some(read::<Count>(&tree, move || counted.set(counted.get() + 1)));

        tree.push::<Count>(1);

        assert!(second.borrow().is_none());
        assert_eq!(late.get(), 0, "the reader was redrawn after its Subscription was dropped");
    }

    #[test]
    fn a_reader_whose_cell_was_seeded_over_is_not_redrawn() {
        let tree = drawn();
        let late = Rc::new(StdCell::new(0));

        let held = tree.scope();
        let _first = read::<Count>(&tree, move || held.seed(Count(0)));
        let counted = late.clone();
        let _second = read::<Count>(&tree, move || counted.set(counted.get() + 1));

        tree.push::<Count>(1);

        assert_eq!(late.get(), 0, "seed drops a cell's readers, yet this one was redrawn");
    }

    /// A redraw that sends to an actor, whose handler pushes to the cell the
    /// reader reads: the turn the actor opens ends inside the redraw.
    fn feeding_itself(through_a_turn: bool) -> usize {
        let scope = drawn();
        let runs = Rc::new(StdCell::new(0));

        let held = scope.scope();
        let _sub = read::<Count>(&scope, {
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
    fn a_redraw_feeding_itself_stops_at_the_limit() {
        assert_eq!(feeding_itself(false), super::SETTLE_ROUNDS);
    }

    #[test]
    fn a_redraw_feeding_itself_through_a_turn_stops_at_the_same_limit() {
        assert_eq!(
            feeding_itself(true),
            super::SETTLE_ROUNDS,
            "the turn ending inside the redraw did not start a drain of its own"
        );
    }
}
