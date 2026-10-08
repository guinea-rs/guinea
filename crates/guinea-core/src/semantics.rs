//! What an element means, for whatever reads it without seeing it: a test
//! now, a screen reader later.
//!
//! An element that draws for itself - rows painted on a composition host -
//! leaves nothing in the native tree under it. It says what it drew as a
//! [`Semantic`] tree instead, and the backend's harness reads that tree where
//! the element is, as it reads native elements.

use std::fmt;
use std::rc::Rc;

use crate::mark::Mark;

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    Group,
    Text,
    Button,
    Image,
    List,
    ListItem,
    Table,
    Row,
    Cell,
}

/// Where a node is, in the coordinates of the element that published it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Its middle.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// One node of what an element drew, and what it holds.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Semantic {
    pub role: Role,
    /// What tells it from its siblings for as long as it lives: a row's key,
    /// not its index.
    pub key: Option<u64>,
    pub text: Option<String>,
    /// Its mark - the `AutomationId` on WinUI.
    pub mark: Option<&'static str>,
    /// What it says when pointed at.
    pub tip: Option<String>,
    pub bounds: Option<Bounds>,
    pub children: Children,
}

/// What a node holds: nodes given outright, or a list's items, built when
/// read.
#[derive(Clone, Debug)]
pub enum Children {
    Nodes(Vec<Semantic>),
    Items(Items),
}

impl Semantic {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            key: None,
            text: None,
            mark: None,
            tip: None,
            bounds: None,
            children: Children::Nodes(Vec::new()),
        }
    }

    pub fn key(self, key: u64) -> Self {
        Self {
            key: Some(key),
            ..self
        }
    }

    pub fn text(self, text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..self
        }
    }

    pub fn mark(self, mark: impl Mark) -> Self {
        Self {
            mark: Some(mark.name()),
            ..self
        }
    }

    pub fn tip(self, tip: impl Into<String>) -> Self {
        Self {
            tip: Some(tip.into()),
            ..self
        }
    }

    pub fn bounds(self, bounds: Bounds) -> Self {
        Self {
            bounds: Some(bounds),
            ..self
        }
    }

    /// Holds `children`, in order, in place of what it held.
    pub fn children(self, children: impl IntoIterator<Item = Semantic>) -> Self {
        Self {
            children: Children::Nodes(children.into_iter().collect()),
            ..self
        }
    }

    /// Holds `items` in place of what it held: it is a list.
    pub fn items(self, items: Items) -> Self {
        Self {
            children: Children::Items(items),
            ..self
        }
    }
}

/// A list's items, none of them built until read: how many there are, item
/// `index` as a node, and how to scroll item `index` into view - which a
/// pointer needs before it can land on one.
#[derive(Clone)]
pub struct Items {
    len: usize,
    item: Rc<dyn Fn(usize) -> Semantic>,
    show: Rc<dyn Fn(usize)>,
}

impl Items {
    pub fn new(
        len: usize,
        item: impl Fn(usize) -> Semantic + 'static,
        show: impl Fn(usize) + 'static,
    ) -> Self {
        Self {
            len,
            item: Rc::new(item),
            show: Rc::new(show),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Item `index`, built now.
    pub fn item(&self, index: usize) -> Semantic {
        (self.item)(index)
    }

    /// Scrolls item `index` into view.
    pub fn show(&self, index: usize) {
        (self.show)(index)
    }
}

impl fmt::Debug for Items {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Items").field("len", &self.len).finish_non_exhaustive()
    }
}
