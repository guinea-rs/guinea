//! What an element that draws for itself says it drew.
//!
//! A table painted on a composition host leaves no XAML under its element, so
//! a test finds nothing there. [`publish`] gives the element a [`Semantic`]
//! tree, and the harness reads it in place of what is not there: its nodes
//! are found, listed as items and clicked like native ones.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use windows_reactor::{ElementRef, ObjectId};

pub use guinea_core::semantics::{Bounds, Children, Items, Role, Semantic};

thread_local! {
    static PUBLISHED: RefCell<Vec<Weak<Entry>>> = const { RefCell::new(Vec::new()) };
}

#[cfg_attr(not(feature = "harness"), allow(dead_code))]
struct Entry {
    owner: Cell<Option<u64>>,
    element: Box<dyn Fn() -> Option<ObjectId>>,
    tree: Box<dyn Fn() -> Semantic>,
}

/// Says that `element` drew what `tree` says, for as long as the returned
/// [`Published`] is kept. `tree` is asked whenever the tree is read, so it
/// says what is drawn then.
///
/// Bounds are in `element`'s own coordinates: where a pointer event on it,
/// or on an element around it of the same size, says it landed.
pub fn publish<T: 'static>(
    element: &ElementRef<T>,
    tree: impl Fn() -> Semantic + 'static,
) -> Published {
    let element = element.clone();
    let entry = Rc::new(Entry {
        owner: Cell::new(None),
        element: Box::new(move || element.get()),
        tree: Box::new(tree),
    });

    PUBLISHED.with(|published| {
        let mut published = published.borrow_mut();
        published.retain(|entry| entry.strong_count() > 0);
        published.push(Rc::downgrade(&entry));
    });

    Published { _entry: entry }
}

/// A [`publish`]ed tree. Dropping it takes the tree back.
#[must_use = "the tree is taken back when this is dropped"]
pub struct Published {
    _entry: Rc<Entry>,
}

#[cfg(feature = "harness")]
fn live() -> Vec<Rc<Entry>> {
    PUBLISHED.with(|published| published.borrow().iter().filter_map(Weak::upgrade).collect())
}

/// Gives host `owner` every published tree whose element is bound and that
/// no host has yet: called as soon as the host has run, so an element it
/// bound is its own before another host runs.
///
/// An element's id means something only in the host that bound it, and the
/// reactor does not say which host that is.
#[cfg(feature = "harness")]
pub(crate) fn claim(owner: u64) {
    for entry in live() {
        if entry.owner.get().is_none() && (entry.element)().is_some() {
            entry.owner.set(Some(owner));
        }
    }
}

#[cfg(feature = "harness")]
fn entry_on(owner: u64, element: ObjectId) -> Option<Rc<Entry>> {
    live()
        .into_iter()
        .rev()
        .find(|entry| entry.owner.get() == Some(owner) && (entry.element)() == Some(element))
}

/// Whether `element` of host `owner` published a tree that is still kept.
#[cfg(feature = "harness")]
pub(crate) fn is_published(owner: u64, element: ObjectId) -> bool {
    entry_on(owner, element).is_some()
}

/// What `element` of host `owner` says it drew, read now.
#[cfg(feature = "harness")]
pub(crate) fn published(owner: u64, element: ObjectId) -> Option<Semantic> {
    entry_on(owner, element).map(|entry| (entry.tree)())
}
