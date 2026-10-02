use std::fmt;

/// Which bus carried a publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Every window hears it.
    Global,
    /// One window's own.
    Window,
}

/// How a stored value changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoreOp {
    Set,
    Delete,
    /// Everything under the path went at once.
    DeletePrefix,
}

/// An observable point, with what identifies it.
///
/// Names are short type names: `processes::Kill`, not a crate path.
#[derive(Clone, Debug, PartialEq)]
pub enum Point {
    /// The UI asked a feature for something.
    Action { message: &'static str },
    /// A message was queued for an actor.
    Send {
        actor: &'static str,
        message: &'static str,
    },
    /// An actor handled a message.
    Handle {
        actor: &'static str,
        message: &'static str,
    },
    /// An actor started background work whose result will come back as
    /// `output`. `actor` is the address it will come back to, by the id the
    /// snapshot lists it under: what a task belongs to is what that actor
    /// belongs to.
    Spawn {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
    },
    /// Background work finished, and its result is on its way to the actor.
    Settled {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
        took_us: u64,
    },
    /// Background work was dropped where it last awaited, because the actor
    /// that started it is gone. Nothing comes back.
    Cancelled {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
        took_us: u64,
    },
    /// An actor opened a source whose items will come to it as `output`.
    /// Recorded under what opened it, which is done with it once it is open:
    /// the items are not its work.
    Source {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
    },
    /// An item came from a source, as `output`. A root, as a timer's tick
    /// is; `source` is the id of the [`Point::Source`] it came from.
    Arrived {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
        source: u64,
    },
    /// A source ended: it ran dry, or `gone` - the actor that opened it is
    /// gone and the source was dropped where it last awaited.
    Closed {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
        took_us: u64,
        gone: bool,
    },
    /// An event went out.
    Publish {
        event: &'static str,
        bus: Bus,
        subscribers: usize,
    },
    /// An event reached a subscriber that is not an actor.
    Deliver { event: &'static str, bus: Bus },
    /// A reducer was changed.
    Push { reducer: &'static str },
    /// A router moved.
    Navigate { root: String, to: String },
    /// A timer fired; `timer` is its id, as the application's timers list
    /// it.
    Tick { timer: u64 },
    /// A persisted value changed.
    Store {
        op: StoreOp,
        path: String,
        /// The declared field the path belongs to, `Settings.theme`, when
        /// the store knows it.
        field: Option<String>,
        /// Whether the change came from outside the process, an edited file.
        outside: bool,
    },
    /// A page or layout drew itself, and how long that took. Recorded only
    /// for the frames worth looking at - see `devtools::rendering`.
    Render {
        segment: &'static str,
        took_us: u64,
    },
    /// An ordinary `tracing` event the application wrote.
    Log {
        level: tracing::Level,
        target: &'static str,
        /// Where it was written, as the compiler named the file: relative to
        /// the workspace root for the application's own crates.
        file: Option<&'static str>,
        line: Option<u32>,
        module: Option<&'static str>,
        /// The message, then the other fields as `name=value`.
        text: String,
    },
    /// A `tracing` span the application opened - an `#[instrument]`ed
    /// function, say. Open until the span closes, and current while it is
    /// entered; what it took is the time it was entered, not the time it
    /// waited in between.
    Span {
        name: &'static str,
        /// The level it was opened at, as [`Point::Log`]'s.
        level: tracing::Level,
        target: &'static str,
        /// Where it was written; see [`Point::Log`].
        file: Option<&'static str>,
        line: Option<u32>,
        module: Option<&'static str>,
        /// Its fields as `name=value`, as they were when it opened.
        fields: String,
    },
    /// Anything else worth a line.
    Note(String),
}

impl Point {
    /// A short name for the kind of point, for filtering.
    pub fn kind(&self) -> &'static str {
        match self {
            Point::Action { .. } => "action",
            Point::Send { .. } => "send",
            Point::Handle { .. } => "handle",
            Point::Spawn { .. } => "spawn",
            Point::Settled { .. } => "settled",
            Point::Cancelled { .. } => "cancelled",
            Point::Source { .. } => "source",
            Point::Arrived { .. } => "arrived",
            Point::Closed { .. } => "closed",
            Point::Publish { .. } => "publish",
            Point::Deliver { .. } => "deliver",
            Point::Push { .. } => "push",
            Point::Navigate { .. } => "navigate",
            Point::Tick { .. } => "tick",
            Point::Store { .. } => "store",
            Point::Render { .. } => "render",
            Point::Log { .. } => "log",
            Point::Span { .. } => "span",
            Point::Note(_) => "note",
        }
    }
}

impl fmt::Display for Bus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Bus::Global => "global",
            Bus::Window => "window",
        })
    }
}

impl fmt::Display for StoreOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StoreOp::Set => "set",
            StoreOp::Delete => "delete",
            StoreOp::DeletePrefix => "delete_prefix",
        })
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Point::Action { message } => write!(f, "action {message}"),
            Point::Send { actor, message } => write!(f, "send {message} → {actor}"),
            Point::Handle { actor, message, .. } => write!(f, "{actor} handles {message}"),
            Point::Spawn { actor, output, .. } => write!(f, "{actor} starts work for {output}"),
            Point::Settled {
                actor,
                output,
                took_us,
                ..
            } => write!(
                f,
                "{actor} has its {output} after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Cancelled {
                actor,
                output,
                took_us,
                ..
            } => write!(
                f,
                "{actor} is gone: {output} cancelled after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Source { actor, output, .. } => write!(f, "{actor} opens a source of {output}"),
            Point::Arrived { actor, output, .. } => write!(f, "{output} arrives at {actor}"),
            Point::Closed {
                actor,
                output,
                took_us,
                gone: true,
                ..
            } => write!(
                f,
                "{actor} is gone: its source of {output} closed after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Closed {
                actor,
                output,
                took_us,
                ..
            } => write!(
                f,
                "{actor}'s source of {output} ran dry after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Publish {
                event,
                bus,
                subscribers,
            } => write!(f, "publish {event} on the {bus} bus to {subscribers}"),
            Point::Deliver { event, bus } => write!(f, "deliver {event} from the {bus} bus"),
            Point::Push { reducer } => write!(f, "push into {reducer}"),
            Point::Navigate { root, to } => write!(f, "{root} navigates to {to}"),
            Point::Tick { timer } => write!(f, "timer #{timer}"),
            Point::Render { segment, took_us } => {
                write!(f, "{segment} drew itself in {:.1} ms", *took_us as f64 / 1000.0)
            }
            Point::Store {
                op,
                path,
                field,
                outside,
            } => {
                let who = if *outside { "disk" } else { "store" };
                let verb = match op {
                    StoreOp::Set => "sets",
                    StoreOp::Delete => "deletes",
                    StoreOp::DeletePrefix => "clears",
                };
                write!(f, "{who} {verb} {path}")?;
                match field {
                    Some(field) => write!(f, " ({field})"),
                    None => Ok(()),
                }
            }
            Point::Log {
                level,
                target,
                text,
                ..
            } => write!(f, "{level} {target}: {text}"),
            Point::Span { name, fields, .. } if fields.is_empty() => f.write_str(name),
            Point::Span { name, fields, .. } => write!(f, "{name} {fields}"),
            Point::Note(text) => f.write_str(text),
        }
    }
}
