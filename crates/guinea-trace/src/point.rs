use std::fmt;

/// An observable point, with what identifies it.
///
/// Names are short type names: `processes::Kill`, not a crate path. What
/// guinea itself does not know - a plugin's own work - is a
/// [`Point::Plugin`], so a match on it names every kind there is.
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
    /// that started it is gone or its lane was cancelled. Nothing comes back.
    Cancelled {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
        took_us: u64,
    },
    /// Background work in a lane gave way to newer work there: it was dropped
    /// unstarted, or cut off, or what it returned came too late to count.
    /// Nothing comes back.
    Superseded {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
        took_us: u64,
    },
    /// Background work was turned away unstarted: its lane was busy and
    /// takes nothing new until it is not.
    Skipped {
        actor: &'static str,
        actor_id: u64,
        output: &'static str,
    },
    /// A source was opened, whose items will come to `to` - an actor, or `fn`
    /// - as `output`; a timer is one. Recorded under what opened it, which is
    /// done with it once it is open: the items are not its work. `source` is
    /// its id, as the scope that holds it lists it, and `file` and `line` say
    /// where it was opened.
    Source {
        source: u64,
        to: &'static str,
        output: &'static str,
        file: &'static str,
        line: u32,
    },
    /// An item came from the source `source`, as `output`, with the name the
    /// source was given, if any. A root of its own.
    Arrived {
        source: u64,
        name: Option<&'static str>,
        to: &'static str,
        output: &'static str,
    },
    /// The source `source` is making its next item, which will come as
    /// `output`: what its stream does meanwhile is recorded under this. A
    /// root, as [`Point::Arrived`] is. Open from when the source is asked for
    /// the item until it has it, runs dry or is dropped.
    Pull {
        source: u64,
        to: &'static str,
        output: &'static str,
    },
    /// The source `source` ended: it ran dry, or `gone` - what held it is
    /// gone, or it was replaced, and it was dropped where it last awaited.
    Closed {
        source: u64,
        to: &'static str,
        output: &'static str,
        took_us: u64,
        gone: bool,
    },
    /// An event went out.
    Publish {
        event: &'static str,
        subscribers: usize,
    },
    /// An event reached a subscriber that is not an actor.
    Deliver { event: &'static str },
    /// A reducer was changed.
    Push { reducer: &'static str },
    /// A router moved.
    Navigate { root: String, to: String },
    /// A page or layout drew itself, and how long that took. Recorded while
    /// devtools watch, for a draw that took `GUINEA_TRACE_RENDER_MS` or
    /// longer, 2 unless set; for every draw under a test.
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
    /// Something a plugin recorded about its own work: a value it stored, a
    /// request it made. `kind` says what, `store.set`; `fields` say the rest,
    /// by name.
    Plugin {
        kind: &'static str,
        fields: Vec<(&'static str, String)>,
    },
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
            Point::Superseded { .. } => "superseded",
            Point::Skipped { .. } => "skipped",
            Point::Source { .. } => "source",
            Point::Arrived { .. } => "arrived",
            Point::Pull { .. } => "pull",
            Point::Closed { .. } => "closed",
            Point::Publish { .. } => "publish",
            Point::Deliver { .. } => "deliver",
            Point::Push { .. } => "push",
            Point::Navigate { .. } => "navigate",
            Point::Render { .. } => "render",
            Point::Log { .. } => "log",
            Point::Span { .. } => "span",
            Point::Plugin { kind, .. } => kind,
        }
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
                "{actor}'s work for {output} cancelled after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Superseded {
                actor,
                output,
                took_us,
                ..
            } => write!(
                f,
                "{actor}'s work for {output} gave way to newer work after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Skipped { actor, output, .. } => {
                write!(f, "{actor} was busy: work for {output} turned away")
            }
            Point::Source { to, output, .. } => write!(f, "a source of {output} opens for {to}"),
            Point::Arrived {
                name: Some(name),
                output,
                to,
                ..
            } => write!(f, "{name}: {output} arrives at {to}"),
            Point::Arrived { output, to, .. } => write!(f, "{output} arrives at {to}"),
            Point::Pull { source, output, .. } => {
                write!(f, "source #{source} makes the next {output}")
            }
            Point::Closed {
                to,
                output,
                took_us,
                gone: true,
                ..
            } => write!(
                f,
                "{to}'s source of {output} dropped after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Closed {
                to,
                output,
                took_us,
                ..
            } => write!(
                f,
                "{to}'s source of {output} ran dry after {:.1} ms",
                *took_us as f64 / 1000.0
            ),
            Point::Publish { event, subscribers } => write!(f, "publish {event} to {subscribers}"),
            Point::Deliver { event } => write!(f, "deliver {event}"),
            Point::Push { reducer } => write!(f, "push into {reducer}"),
            Point::Navigate { root, to } => write!(f, "{root} navigates to {to}"),
            Point::Render { segment, took_us } => {
                write!(f, "{segment} drew itself in {:.1} ms", *took_us as f64 / 1000.0)
            }
            Point::Log {
                level,
                target,
                text,
                ..
            } => write!(f, "{level} {target}: {text}"),
            Point::Span { name, fields, .. } if fields.is_empty() => f.write_str(name),
            Point::Span { name, fields, .. } => write!(f, "{name} {fields}"),
            Point::Plugin { kind, fields } if fields.is_empty() => f.write_str(kind),
            Point::Plugin { kind, fields } => write!(f, "{kind} {}", Fields(fields)),
        }
    }
}

/// A plugin point's fields as `name=value`, a space between each.
pub(crate) struct Fields<'a>(pub(crate) &'a [(&'static str, String)]);

impl fmt::Display for Fields<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (at, (name, value)) in self.0.iter().enumerate() {
            if at > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{name}={value}")?;
        }
        Ok(())
    }
}
