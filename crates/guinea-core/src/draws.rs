//! The one contract between a scope tree and whatever draws it.
//!
//! A backend that keeps what it drew - a reconciler, a toolkit with
//! properties - says once per tree how it redraws: [`Scope::drawn_by`]. What
//! it drew from a reducer it registers as a reader of that reducer, with
//! [`ReducerBinding::read_by`](crate::binding::ReducerBinding::read_by), and
//! the tree keeps who reads what. After a turn's state has settled the tree
//! hands its [`Draws`] every reader of what moved, in one call.
//!
//! A backend that reads everything again each frame needs none of this.

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::scope::{Scope, Watcher};
use crate::trace::{self, Cause};

/// How a tree is redrawn, and what it is told to redraw.
///
/// ```
/// # use std::cell::RefCell;
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
/// struct Labels(Rc<RefCell<Vec<Vec<&'static str>>>>);
///
/// impl Draws for Labels {
///     type Reader = &'static str;
///
///     fn redraw(&self, readers: Readers<'_, &'static str>) {
///         self.0.borrow_mut().push(readers.copied().collect());
///     }
/// }
///
/// let tree = ScopeTree::new();
/// let redrawn = Rc::new(RefCell::new(Vec::new()));
/// tree.drawn_by(Labels(redrawn.clone()));
///
/// let _title = tree.binding::<Count>().read_by("title");
/// let _badge = tree.binding::<Count>().read_by("badge");
///
/// notify::turn(|| {
///     tree.push::<Count>(1);
///     tree.push::<Count>(2);
/// });
///
/// assert_eq!(*redrawn.borrow(), [vec!["title", "badge"]]);
/// ```
pub trait Draws: 'static {
    /// What the backend registers for each thing it drew from a reducer: a
    /// component to refresh, a property to set again.
    type Reader: 'static;

    /// Redraws `readers`, each a reader of a reducer that moved in the turn
    /// that just ended, each once, in the order the reducers first moved.
    fn redraw(&self, readers: Readers<'_, Self::Reader>);
}

/// The readers one drain hands a tree's [`Draws`].
///
/// A reader whose scope went, or that was dropped, while the ones before it
/// were redrawn is skipped.
pub struct Readers<'a, R> {
    list: std::slice::Iter<'a, Read>,
    /// What moved the reducer the current reader reads: what redrawing it
    /// sets off is still that cause's doing.
    under: Option<trace::Resumed>,
    of: PhantomData<fn() -> R>,
}

impl<'a, R: 'static> Iterator for Readers<'a, R> {
    type Item = &'a R;

    fn next(&mut self) -> Option<&'a R> {
        self.under = None;

        for read in self.list.by_ref() {
            if !read.scope.is_alive() || read.reader.is_gone() {
                continue;
            }
            let Some(reader) = read.reader.run.downcast_ref::<R>() else {
                continue;
            };

            self.under = Some(trace::resume(read.cause));
            return Some(reader);
        }

        None
    }
}

/// A reader as a drain hands it on: where it reads, and what moved it.
pub(crate) struct Read {
    pub(crate) scope: Scope,
    pub(crate) reader: Rc<Watcher<dyn Any>>,
    pub(crate) cause: Option<Cause>,
}

/// A tree's [`Draws`], whatever its reader.
pub(crate) trait Drawing {
    fn reader(&self) -> TypeId;

    fn redraw(&self, readers: &[Read]);
}

struct Drawn<D>(D);

impl<D: Draws> Drawing for Drawn<D> {
    fn reader(&self) -> TypeId {
        TypeId::of::<D::Reader>()
    }

    fn redraw(&self, readers: &[Read]) {
        self.0.redraw(Readers {
            list: readers.iter(),
            under: None,
            of: PhantomData,
        });
    }
}

impl Scope {
    /// Makes `draws` what redraws this scope's tree, in place of what did.
    ///
    /// Any scope of the tree names it: a window's scope draws the whole tree
    /// it is in.
    ///
    /// ```
    /// # use std::cell::Cell;
    /// # use std::rc::Rc;
    /// # use guinea_core::draws::{Draws, Readers};
    /// # use guinea_core::scope::ScopeTree;
    /// # #[derive(Clone, Default, Debug)]
    /// # struct Count(u32);
    /// # #[guinea_macros::reducer]
    /// # fn count(this: &mut Count, to: u32) { this.0 = to; }
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
    /// let window = tree.child();
    /// let (first, second) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    ///
    /// window.drawn_by(Counting(first.clone()));
    /// let _reading = tree.binding::<Count>().read_by(());
    /// tree.push::<Count>(1);
    ///
    /// window.drawn_by(Counting(second.clone()));
    /// tree.push::<Count>(2);
    ///
    /// assert_eq!((first.get(), second.get()), (1, 1));
    /// ```
    ///
    /// # Panics
    ///
    /// When the tree is gone.
    pub fn drawn_by(&self, draws: impl Draws) {
        self.set_tree_drawing(Rc::new(Drawn(draws)));
    }
}

/// Draws a tree whose readers are closures, by calling them.
#[cfg(test)]
pub(crate) struct Calls;

#[cfg(test)]
impl Draws for Calls {
    type Reader = Box<dyn Fn()>;

    fn redraw(&self, readers: Readers<'_, Box<dyn Fn()>>) {
        for redraw in readers {
            redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::scope::ScopeTree;

    #[derive(Clone, Default, Debug)]
    struct Count(u32);

    #[guinea_macros::reducer]
    fn count(this: &mut Count, to: u32) {
        this.0 = to;
    }

    #[derive(Clone, Default, Debug)]
    struct Mirror(u32);

    #[guinea_macros::reducer]
    fn mirror(this: &mut Mirror, to: u32) {
        this.0 = to;
    }

    /// Writes down each redraw it is asked for, as the names it was handed.
    struct Names(Rc<RefCell<Vec<Vec<&'static str>>>>);

    impl Draws for Names {
        type Reader = &'static str;

        fn redraw(&self, readers: Readers<'_, &'static str>) {
            self.0.borrow_mut().push(readers.copied().collect());
        }
    }

    fn drawn(tree: &ScopeTree) -> Rc<RefCell<Vec<Vec<&'static str>>>> {
        let redrawn = Rc::default();
        tree.drawn_by(Names(Rc::clone(&redrawn)));
        redrawn
    }

    #[test]
    fn a_turn_redraws_every_reader_of_what_moved_in_one_call() {
        let tree = ScopeTree::new();
        let redrawn = drawn(&tree);
        let _count = tree.binding::<Count>().read_by("count");
        let _mirror = tree.binding::<Mirror>().read_by("mirror");
        let _also_count = tree.binding::<Count>().read_by("also count");

        crate::notify::turn(|| {
            tree.push::<Mirror>(1);
            tree.push::<Count>(2);
            tree.push::<Mirror>(3);
        });

        assert_eq!(*redrawn.borrow(), [vec!["mirror", "count", "also count"]]);
    }

    #[test]
    fn a_reader_let_go_of_is_not_redrawn() {
        let tree = ScopeTree::new();
        let redrawn = drawn(&tree);
        let _kept = tree.binding::<Count>().read_by("kept");
        drop(tree.binding::<Count>().read_by("let go"));

        tree.push::<Count>(1);

        assert_eq!(*redrawn.borrow(), [vec!["kept"]]);
    }

    #[test]
    fn a_reader_whose_scope_went_while_the_ones_before_it_were_redrawn_is_skipped() {
        struct Closing(Scope, Rc<RefCell<Vec<&'static str>>>);

        impl Draws for Closing {
            type Reader = &'static str;

            fn redraw(&self, readers: Readers<'_, &'static str>) {
                for reader in readers {
                    self.1.borrow_mut().push(reader);
                    if *reader == "closes the page" {
                        self.0.remove();
                    }
                }
            }
        }

        let tree = ScopeTree::new();
        let page = tree.child();
        let redrawn = Rc::default();
        tree.drawn_by(Closing(page, Rc::clone(&redrawn)));
        let _closes = tree.binding::<Count>().read_by("closes the page");
        let _on_the_page = page.binding::<Count>().read_by("on the page");

        crate::notify::turn(|| {
            tree.push::<Count>(1);
            page.push::<Count>(1);
        });

        assert_eq!(*redrawn.borrow(), ["closes the page"]);
    }

    #[test]
    fn a_reader_of_a_tree_nothing_draws_is_refused() {
        let tree = ScopeTree::new();
        let scope = tree.scope();

        let read = std::panic::catch_unwind(|| drop(scope.binding::<Count>().read_by("count")));

        assert!(read.is_err(), "a reader nothing would ever redraw was taken");
    }

    #[test]
    fn a_reader_of_another_kind_than_the_tree_is_drawn_with_is_refused() {
        let tree = ScopeTree::new();
        let _redrawn = drawn(&tree);
        let scope = tree.scope();

        let read = std::panic::catch_unwind(|| drop(scope.binding::<Count>().read_by(7_u32)));

        assert!(read.is_err(), "a reader the tree's Draws cannot be handed was taken");
    }
}
