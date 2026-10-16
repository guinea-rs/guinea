//! What devtools show of a state: a node at a time, as deep and as wide as
//! they asked, never the whole of it.
//!
//! A state that implements [`Inspect`] is walked only where it is open: a
//! list of a thousand items read through a window of fifty costs fifty, and
//! every node closed to the viewer costs only its one-line preview, which has
//! a budget of [`PREVIEW`] bytes. One without falls back on `Debug`, whose
//! text is kept to the same budget - but a `Debug` of a collection still steps
//! through all of it, only without writing, so a large state wants
//! [`Inspect`]. One with neither shows its type's name.

use std::fmt::{self, Debug, Write as _};
use std::ops::Range;

mod std_impls;

/// How long a preview may grow before it is cut, in bytes.
pub(crate) const PREVIEW: usize = 120;

/// How many items of a list or map are read when no window was asked for.
pub(crate) const WINDOW: usize = 100;

/// How long the text of an open state shown through `Debug` may grow, in
/// bytes.
pub(crate) const TEXT: usize = 64 * 1024;

/// A value devtools can open a node at a time.
///
/// `#[derive(Inspect)]` writes it; by hand it is one call on `at`:
///
/// ```
/// use guinea_core::inspect::{Inspect, Inspector, Node};
///
/// struct Process {
///     pid: u32,
///     name: String,
/// }
///
/// impl Inspect for Process {
///     fn inspect(&self, at: Inspector<'_>) {
///         at.record("Process")
///             .field("pid", &self.pid)
///             .field("name", &self.name)
///             .finish();
///     }
/// }
///
/// let shown = Node::of(&Process { pid: 4, name: "System".into() }, None);
/// assert_eq!(shown.preview, r#"Process { pid: 4, name: "System" }"#);
/// ```
pub trait Inspect {
    fn inspect(&self, at: Inspector<'_>);
}

/// One step down from a node to a child.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// A field of a record, or `"0"`, `"1"`… of a tuple.
    Field(String),
    /// An item of a list, by position.
    Index(usize),
    /// An entry of a map, by its key's preview.
    Key(String),
}

/// What the viewer opened: this node, the window of it they look through,
/// and which of its children are open too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Open {
    /// Which items of a list or map, by position; the first [`WINDOW`] when
    /// `None`.
    pub window: Option<Range<usize>>,
    pub children: Vec<(Step, Open)>,
}

impl Open {
    /// The child at `step`, when it is open.
    pub fn child(&self, step: &Step) -> Option<&Open> {
        self.children
            .iter()
            .find(|(open, _)| open == step)
            .map(|(_, open)| open)
    }

    fn field(&self, name: &str) -> Option<&Open> {
        self.children.iter().find_map(|(step, open)| match step {
            Step::Field(field) if field == name => Some(open),
            _ => None,
        })
    }

    fn window(&self, len: usize) -> Range<usize> {
        let window = self.window.clone().unwrap_or(0..WINDOW);
        window.start.min(len)..window.end.min(len)
    }
}

/// What a node is, for how devtools draw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Nothing to open: a number, a string, a unit variant.
    Value,
    /// Named fields, or a tuple's.
    Record,
    List { len: usize },
    Map { len: usize },
    /// Shown through `Debug`: opening it gives its text, kept to [`TEXT`].
    Text,
    /// Neither `Inspect` nor `Debug`: only its type's name.
    Opaque,
    /// Not readable now: the actor is handling a message, or the state is
    /// being changed.
    Busy,
}

/// A node as devtools are sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub form: Form,
    /// One line, at most [`PREVIEW`] bytes and an ellipsis.
    pub preview: String,
    /// What is in it, when it was asked for open.
    pub opened: Option<Opened>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    Children(Vec<(Step, Node)>),
    Text(String),
}

impl Node {
    /// `value` read as far as `open` reaches.
    pub fn of(value: &dyn Inspect, open: Option<&Open>) -> Node {
        Self::shown(|at| value.inspect(at), open)
    }

    /// What `show` writes, read as far as `open` reaches: for a value shown
    /// by something other than its own [`Inspect`].
    pub fn shown(show: impl Fn(Inspector<'_>), open: Option<&Open>) -> Node {
        let mut form = Form::Value;
        let mut preview = Preview::new(PREVIEW);
        show(Inspector {
            to: To::Preview {
                text: &mut preview,
                form: Some(&mut form),
            },
        });

        let opened = open.and_then(|open| {
            let mut opened = None;
            show(Inspector {
                to: To::Open {
                    open,
                    opened: &mut opened,
                },
            });
            opened
        });

        Node {
            form,
            preview: preview.text,
            opened,
        }
    }

    /// A node that cannot be read now, saying why.
    pub fn busy(why: &str) -> Node {
        Node {
            form: Form::Busy,
            preview: why.to_string(),
            opened: None,
        }
    }
}

/// Where a value writes itself: one of its methods, called once.
#[must_use = "a value shows nothing until it calls one of its methods"]
pub struct Inspector<'a> {
    to: To<'a>,
}

enum To<'a> {
    Preview {
        text: &'a mut Preview,
        form: Option<&'a mut Form>,
    },
    Open {
        open: &'a Open,
        opened: &'a mut Option<Opened>,
    },
}

impl<'a> Inspector<'a> {
    /// A value with nothing in it to open, written by its `Debug`.
    pub fn value(self, value: &dyn Debug) {
        if let To::Preview { text, form } = self.to {
            set(form, Form::Value);
            let _ = write!(text, "{value:?}");
        }
    }

    /// A value only `Debug` can show: previewed on one line, and opened as
    /// its pretty text, kept to [`TEXT`].
    pub fn debug(self, value: &dyn Debug) {
        match self.to {
            To::Preview { text, form } => {
                set(form, Form::Text);
                let _ = write!(text, "{value:?}");
            }
            To::Open { opened, .. } => {
                let mut text = Preview::new(TEXT);
                let _ = write!(text, "{value:#?}");
                *opened = Some(Opened::Text(text.text));
            }
        }
    }

    /// A value that cannot be shown, under the name of its type.
    pub fn opaque(self, name: &str) {
        if let To::Preview { text, form } = self.to {
            set(form, Form::Opaque);
            let _ = text.write_str(name);
        }
    }

    /// A unit struct or variant.
    pub fn unit(self, name: &str) {
        if let To::Preview { text, form } = self.to {
            set(form, Form::Value);
            let _ = text.write_str(name);
        }
    }

    /// A struct or variant with named fields; `.field(..)` each, then
    /// `.finish()`.
    pub fn record(self, name: &'a str) -> Record<'a> {
        Record::new(self.to, name, false)
    }

    /// A tuple struct or variant; `name` is empty for a bare tuple.
    pub fn tuple(self, name: &'a str) -> Record<'a> {
        Record::new(self.to, name, true)
    }

    /// `len` items, of which only those in view are taken from `items`.
    pub fn list<'v, T: Inspect + 'v>(self, len: usize, items: impl Iterator<Item = &'v T>) {
        match self.to {
            To::Preview { text, form } => {
                set(form, Form::List { len });

                let _ = text.write_str("[");
                for (at, item) in items.enumerate() {
                    if text.cut {
                        return;
                    }
                    if at > 0 {
                        let _ = text.write_str(", ");
                    }
                    item.inspect(Inspector::preview(&mut *text));
                }
                let _ = text.write_str("]");
            }
            To::Open { open, opened } => {
                let window = open.window(len);

                let children = items
                    .skip(window.start)
                    .take(window.len())
                    .zip(window)
                    .map(|(item, index)| {
                        let step = Step::Index(index);
                        let node = Node::of(item, open.child(&step));
                        (step, node)
                    })
                    .collect();
                *opened = Some(Opened::Children(children));
            }
        }
    }

    /// `len` entries, of which only those in view are taken from `entries`.
    pub fn map<'v, K: Inspect + 'v, V: Inspect + 'v>(
        self,
        len: usize,
        entries: impl Iterator<Item = (&'v K, &'v V)>,
    ) {
        match self.to {
            To::Preview { text, form } => {
                set(form, Form::Map { len });

                let _ = text.write_str("{");
                for (at, (key, value)) in entries.enumerate() {
                    if text.cut {
                        return;
                    }
                    if at > 0 {
                        let _ = text.write_str(", ");
                    }
                    key.inspect(Inspector::preview(&mut *text));
                    let _ = text.write_str(": ");
                    value.inspect(Inspector::preview(&mut *text));
                }
                let _ = text.write_str("}");
            }
            To::Open { open, opened } => {
                let window = open.window(len);

                let children = entries
                    .skip(window.start)
                    .take(window.len())
                    .map(|(key, value)| {
                        let step = Step::Key(Node::of(key, None).preview);
                        let node = Node::of(value, open.child(&step));
                        (step, node)
                    })
                    .collect();
                *opened = Some(Opened::Children(children));
            }
        }
    }

    fn preview(text: &'a mut Preview) -> Self {
        Inspector {
            to: To::Preview { text, form: None },
        }
    }
}

fn set(form: Option<&mut Form>, to: Form) {
    if let Some(form) = form {
        *form = to;
    }
}

/// A record or tuple being written. See [`Inspector::record`].
#[must_use = "a record is written by `.finish()`"]
pub struct Record<'a> {
    to: To<'a>,
    name: &'a str,
    tuple: bool,
    fields: usize,
    children: Vec<(Step, Node)>,
}

impl<'a> Record<'a> {
    fn new(mut to: To<'a>, name: &'a str, tuple: bool) -> Self {
        if let To::Preview { form, .. } = &mut to {
            set(form.as_deref_mut(), Form::Record);
        }

        Record {
            to,
            name,
            tuple,
            fields: 0,
            children: Vec::new(),
        }
    }

    /// The next field: named in a record, counted in a tuple.
    pub fn field(mut self, name: &str, value: &dyn Inspect) -> Self {
        match &mut self.to {
            To::Preview { text, .. } => {
                if !text.cut {
                    let opening = match (self.fields, self.tuple) {
                        (0, true) => "(",
                        (0, false) => " { ",
                        _ => ", ",
                    };
                    if self.fields == 0 {
                        let _ = text.write_str(self.name);
                    }
                    let _ = text.write_str(opening);

                    if !self.tuple {
                        let _ = write!(text, "{name}: ");
                    }
                    value.inspect(Inspector::preview(&mut **text));
                }
            }
            To::Open { open, .. } => {
                let node = Node::of(value, open.field(name));
                self.children.push((Step::Field(name.to_string()), node));
            }
        }

        self.fields += 1;
        self
    }

    pub fn finish(self) {
        match self.to {
            To::Preview { text, .. } => {
                let closing = match (self.fields, self.tuple) {
                    (0, _) => self.name,
                    (_, true) => ")",
                    (_, false) => " }",
                };
                let _ = text.write_str(closing);
            }
            To::Open { opened, .. } => {
                *opened = Some(Opened::Children(self.children));
            }
        }
    }
}

/// A preview being written: cut, with an ellipsis, once it is out of bytes.
/// Every write after the cut fails, so a `Debug` stops writing; std's
/// `debug_list` and its kin still step through every entry they are handed.
struct Preview {
    text: String,
    left: usize,
    cut: bool,
}

impl Preview {
    fn new(budget: usize) -> Self {
        Self {
            text: String::new(),
            left: budget,
            cut: false,
        }
    }
}

impl fmt::Write for Preview {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.cut {
            return Err(fmt::Error);
        }

        if s.len() <= self.left {
            self.text.push_str(s);
            self.left -= s.len();
            return Ok(());
        }

        let mut end = self.left;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&s[..end]);
        self.text.push('…');

        self.left = 0;
        self.cut = true;
        Err(fmt::Error)
    }
}

pub(crate) mod pick {
    //! What `actor!` and `#[reducer]` show a state with: its [`Inspect`],
    //! else its `Debug`, else its type's name - picked where the type is
    //! known, by which of three impls method lookup reaches first.

    use super::{Inspect, Inspector};
    use std::fmt::Debug;

    pub struct Probe<'a, T: ?Sized>(pub &'a T);

    pub trait ViaInspect {
        fn show(&self, at: Inspector<'_>);
    }

    impl<T: Inspect + ?Sized> ViaInspect for &&Probe<'_, T> {
        fn show(&self, at: Inspector<'_>) {
            self.0.inspect(at);
        }
    }

    pub trait ViaDebug {
        fn show(&self, at: Inspector<'_>);
    }

    impl<T: Debug + ?Sized> ViaDebug for &Probe<'_, T> {
        fn show(&self, at: Inspector<'_>) {
            at.debug(&self.0);
        }
    }

    pub trait ViaNothing {
        fn show(&self, at: Inspector<'_>);
    }

    impl<T: ?Sized> ViaNothing for Probe<'_, T> {
        fn show(&self, at: Inspector<'_>) {
            at.opaque(crate::short_type_name::<T>());
        }
    }
}

#[cfg(test)]
mod tests;
