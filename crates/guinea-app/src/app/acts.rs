//! What an action set off, as the trace tells it: the harness listens to every
//! point on its thread and can say, for any one of them, whether everything it
//! caused has finished, and what that was.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use guinea_core::actor::short_type_name;
use guinea_core::observability::{EveryRender, record_every_render};
use guinea_core::trace::{self, Cause, Point, Trace};

/// Every point recorded on this thread while it is held.
pub(crate) struct Recorder {
    log: Rc<RefCell<Log>>,
    _every_render: EveryRender,
}

#[derive(Default)]
struct Log {
    order: Vec<Cause>,
    points: HashMap<Cause, Point>,
    children: HashMap<Cause, Vec<Cause>>,
    open: HashSet<Cause>,
}

impl Log {
    fn hear(&mut self, trace: &Trace) {
        match trace {
            Trace::Begin(record) => {
                self.open.insert(record.id);
                self.note(record.id, record.parent, record.point.clone());
            }
            Trace::Mark(record) => self.note(record.id, record.parent, record.point.clone()),
            Trace::End { id, .. } => {
                self.open.remove(id);
            }
        }
    }

    fn note(&mut self, id: Cause, parent: Option<Cause>, point: Point) {
        self.order.push(id);
        self.points.insert(id, point);

        if let Some(parent) = parent {
            self.children.entry(parent).or_default().push(id);
        }
    }

    fn children(&self, of: Cause) -> &[Cause] {
        self.children.get(&of).map(Vec::as_slice).unwrap_or(&[])
    }

    fn ended(&self, spawn: Cause) -> bool {
        self.children(spawn).iter().any(|child| {
            matches!(
                self.points.get(child),
                Some(Point::Settled { .. } | Point::Cancelled { .. })
            )
        })
    }
}

impl Recorder {
    pub(crate) fn start() -> Self {
        let log = Rc::new(RefCell::new(Log::default()));

        let hearing = log.clone();
        trace::observe(move |trace| hearing.borrow_mut().hear(trace));

        Self {
            log,
            _every_render: record_every_render(),
        }
    }

    pub(crate) fn heard(&self) -> usize {
        self.log.borrow().order.len()
    }

    /// The first action recorded at or after `from`, with nothing above it.
    pub(crate) fn action_since(&self, from: usize) -> Option<Cause> {
        let log = self.log.borrow();

        log.order[from..]
            .iter()
            .copied()
            .find(|id| matches!(log.points.get(id), Some(Point::Action { .. })))
    }

    /// Whether everything `cause` set off has finished: no point under it is
    /// still open, and every task it spawned has settled or been cancelled.
    pub(crate) fn is_settled(&self, cause: Cause) -> bool {
        let log = self.log.borrow();
        let mut unseen = vec![cause];

        while let Some(at) = unseen.pop() {
            if log.open.contains(&at) {
                return false;
            }

            if matches!(log.points.get(&at), Some(Point::Spawn { .. })) && !log.ended(at) {
                return false;
            }

            unseen.extend_from_slice(log.children(at));
        }

        true
    }

    pub(crate) fn chain(&self, cause: Cause) -> Chain {
        let log = self.log.borrow();
        Chain::grow(&log, cause)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        trace::stop_observing();
    }
}

/// A point and everything it caused, in the order it happened.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub point: Point,
    pub then: Vec<Chain>,
}

impl Chain {
    fn grow(log: &Log, at: Cause) -> Self {
        Self {
            point: log.points.get(&at).cloned().unwrap_or(Point::Note(String::new())),
            then: log.children(at).iter().map(|child| Self::grow(log, *child)).collect(),
        }
    }

    /// Every point in the chain, this one first, depth first.
    pub fn points(&self) -> Vec<&Point> {
        let mut points = vec![&self.point];
        for next in &self.then {
            points.extend(next.points());
        }
        points
    }

    pub fn has(&self, test: impl Fn(&Point) -> bool) -> bool {
        self.points().into_iter().any(test)
    }

    /// Whether `E` was published somewhere in it.
    pub fn published<E>(&self) -> bool {
        let event = short_type_name::<E>();
        self.has(|point| matches!(point, Point::Publish { event: said, .. } if *said == event))
    }

    /// Whether some actor handled an `M` somewhere in it.
    pub fn handled<M>(&self) -> bool {
        let message = short_type_name::<M>();
        self.has(|point| matches!(point, Point::Handle { message: said, .. } if *said == message))
    }

    /// Whether `R` was updated somewhere in it.
    pub fn pushed<R>(&self) -> bool {
        let reducer = short_type_name::<R>();
        self.has(|point| matches!(point, Point::Push { reducer: said } if *said == reducer))
    }

    /// Whether a task it spawned was cancelled rather than answering.
    pub fn cancelled(&self) -> bool {
        self.has(|point| matches!(point, Point::Cancelled { .. }))
    }

    /// What happened and in what shape, with what differs from one run to the
    /// next left out: ids, times, which instance of an actor, which timer.
    /// What a snapshot of behaviour compares.
    ///
    /// Steps that ran side by side stay in the order they ran, which the seed
    /// decides; [`Shape::unordered`] takes that order out.
    pub fn shape(&self) -> Shape {
        Shape {
            step: Step::of(&self.point),
            then: self.then.iter().map(Chain::shape).collect(),
        }
    }
}

/// A [`Chain`] as a snapshot keeps it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Shape {
    pub step: Step,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub then: Vec<Shape>,
}

impl Shape {
    /// The same shape with the steps under every point sorted, so that work
    /// that ran side by side compares equal whichever went first.
    pub fn unordered(mut self) -> Self {
        self.then = self.then.into_iter().map(Shape::unordered).collect();
        self.then.sort();
        self
    }
}

/// One point of a [`Shape`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Step {
    Action { message: &'static str },
    Send { actor: &'static str, message: &'static str },
    Handle { actor: &'static str, message: &'static str },
    Spawn { actor: &'static str, output: &'static str },
    Settled { actor: &'static str, output: &'static str },
    Cancelled { actor: &'static str, output: &'static str },
    Source { actor: &'static str, output: &'static str },
    Arrived { actor: &'static str, output: &'static str },
    Pull { actor: &'static str, output: &'static str },
    Closed { actor: &'static str, output: &'static str, gone: bool },
    Publish { event: &'static str, bus: &'static str, subscribers: usize },
    Deliver { event: &'static str, bus: &'static str },
    Push { reducer: &'static str },
    Navigate { to: String },
    Tick,
    Store { op: &'static str, path: String, field: Option<String>, outside: bool },
    Render { segment: &'static str },
    Log { level: String, target: &'static str, text: String },
    Span { name: &'static str, fields: String },
    Note(String),
}

impl Step {
    fn of(point: &Point) -> Self {
        let bus = |bus: &guinea_core::trace::Bus| match bus {
            guinea_core::trace::Bus::Global => "global",
            guinea_core::trace::Bus::Window => "window",
        };

        match point {
            Point::Action { message } => Step::Action { message },
            Point::Send { actor, message } => Step::Send { actor, message },
            Point::Handle { actor, message, .. } => Step::Handle { actor, message },
            Point::Spawn { actor, output, .. } => Step::Spawn { actor, output },
            Point::Settled { actor, output, .. } => Step::Settled { actor, output },
            Point::Cancelled { actor, output, .. } => Step::Cancelled { actor, output },
            Point::Source { actor, output, .. } => Step::Source { actor, output },
            Point::Arrived { actor, output, .. } => Step::Arrived { actor, output },
            Point::Pull { actor, output, .. } => Step::Pull { actor, output },
            Point::Closed {
                actor,
                output,
                gone,
                ..
            } => Step::Closed {
                actor,
                output,
                gone: *gone,
            },
            Point::Publish {
                event,
                bus: on,
                subscribers,
            } => Step::Publish {
                event,
                bus: bus(on),
                subscribers: *subscribers,
            },
            Point::Deliver { event, bus: on } => Step::Deliver { event, bus: bus(on) },
            Point::Push { reducer } => Step::Push { reducer },
            Point::Navigate { to, .. } => Step::Navigate { to: to.clone() },
            Point::Tick { .. } => Step::Tick,
            Point::Store {
                op,
                path,
                field,
                outside,
            } => Step::Store {
                op: match op {
                    guinea_core::trace::StoreOp::Set => "set",
                    guinea_core::trace::StoreOp::Delete => "delete",
                    guinea_core::trace::StoreOp::DeletePrefix => "delete prefix",
                },
                path: path.clone(),
                field: field.clone(),
                outside: *outside,
            },
            Point::Render { segment, .. } => Step::Render { segment },
            Point::Log {
                level,
                target,
                text,
                ..
            } => Step::Log {
                level: level.to_string(),
                target,
                text: text.clone(),
            },
            Point::Span { name, fields, .. } => Step::Span {
                name,
                fields: fields.clone(),
            },
            Point::Note(note) => Step::Note(note.clone()),
            point => Step::Note(point.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(step: Step) -> Shape {
        Shape {
            step,
            then: Vec::new(),
        }
    }

    fn under(then: Vec<Shape>) -> Shape {
        Shape {
            step: Step::Action { message: "Refresh" },
            then,
        }
    }

    #[test]
    fn work_that_ran_side_by_side_compares_equal_in_either_order() {
        let pushed = leaf(Step::Push { reducer: "Listing" });
        let published = leaf(Step::Publish {
            event: "Refreshed",
            bus: "global",
            subscribers: 1,
        });

        let one = under(vec![pushed.clone(), published.clone()]);
        let other = under(vec![published, pushed]);

        assert_ne!(one, other);
        assert_eq!(one.unordered(), other.unordered());
    }
}
