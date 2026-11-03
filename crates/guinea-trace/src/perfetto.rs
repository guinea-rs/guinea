//! Streaming native Perfetto traces, without a tracing service or a custom UI.
//!
//! Enable the `perfetto` feature. Attach before starting the workload, stop
//! producers before finishing, and use [`Recording::finish`] to check write errors.
//! Each packet is written immediately; wrapping the writer in `BufWriter` or a
//! compression writer is the caller's choice. Recording is synchronous, so use
//! the same instrumentation when comparing two runs.
//!
//! ```
//! use guinea_trace::{Sink, record_into, enter, Point};
//! use guinea_trace::perfetto::Recording;
//! let sink = Sink::new();
//! let recording = Recording::start(&sink, Vec::new())?;
//! {
//!     let _sink = record_into(&sink);
//!     let _action = enter(|| Point::Action { message: "Refresh" });
//! }
//! let bytes = recording.finish()?;
//! assert!(!bytes.is_empty());
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! Intervals use real monotonic emission times. `guinea_took_ns` on their end
//! preserves Guinea's duration excluding observation overhead; neither is CPU
//! time. Overlapping operations use separate reusable lanes, including spans
//! which close on another thread. Starting midway leaves unmatched ends out;
//! operations still open at finish remain unfinished slices in Perfetto.
//! Use Perfetto v59 or newer. Causal arrows retain the last 16,384 origins;
//! older or unrecorded parents still have their id in `guinea_parent`, but no
//! arrow. No state values or process-memory samples are collected automatically.
//! Through the `guinea` facade, enable its `perfetto` feature and use
//! `guinea::observability::trace::perfetto::Recording`. Get the application's
//! existing sink with `guinea::observability::trace::sink_of(scope)`; creating a
//! new sink records only work explicitly directed into that new sink.
//!
//! Reusable memory queries live in `sql/guinea/memory.sql`. Load the package with
//! `trace_processor --add-sql-package crates/guinea-trace/sql/guinea` and use
//! `INCLUDE PERFETTO MODULE guinea.memory`. `guinea_scope_memory_stats` reports
//! observed growth, allocation turnover and bytes retained after scope closure;
//! `guinea_scope_memory_growth(start_ts, end_ts)` selects a time window. These
//! are allocation-origin metrics, not a proof of unreachable memory. Templates
//! in `sql/memory_metrics.pbtxt` expose the same values to trace summarization.
//!
//! `cargo xtask profile -p your-app --bin your-app --features perfetto` builds
//! with symbols and starts samply. Attach [`from_env`] to the application's
//! existing sink at startup, retain the recording, then call `finish()` after
//! producers stop. Without `GUINEA_TRACE_FILE`, `from_env` does nothing.
//! The launcher combines CPU samples and Guinea events in `combined.pftrace`.
//! `[Guinea]` frames describe causal context above the native sampled stack;
//! they are not physical function calls. Execution transitions delimit each
//! async poll, and samples explicitly reporting zero CPU progress are excluded.
//! On Windows, process creation time anchors the sampler and application clocks.
//! Windows and Linux export native thread IDs for CPU correlation. Other
//! platforms, including macOS, export internal IDs and preserve native CPU
//! stacks without causal attribution. Execution metadata carries `native_id`
//! explicitly (null for internal IDs), so processing on another OS is safe.
//! Sampling may require OS permissions.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use perfetto_protos::clock_snapshot::{ClockSnapshot, clock_snapshot::Clock};
use perfetto_protos::counter_descriptor::CounterDescriptor;
use perfetto_protos::debug_annotation::DebugAnnotation;
use perfetto_protos::process_descriptor::ProcessDescriptor;
use perfetto_protos::thread_descriptor::ThreadDescriptor;
use perfetto_protos::trace::Trace as ProtoTrace;
use perfetto_protos::trace_packet::TracePacket;
use perfetto_protos::track_descriptor::TrackDescriptor;
use perfetto_protos::track_event::{TrackEvent, track_event::Type};
use protobuf::Message;

use crate::{Cause, Point, Record, Sink, Streaming, Trace};

const PARENTS: usize = 16_384;
// BUILTIN_CLOCK_TRACE_FILE, added after the generated schema's release.
// A private file timeline, not the host's boot clock. Perfetto v59+.
const TRACE_CLOCK: u32 = 11;

mod cpu;
pub use cpu::{merge_cpu, merge_cpu_raw};

pub(crate) fn context_changed() {
    crate::observed(crate::sink::context_changed);
}

/// Attach the recording requested by `cargo xtask profile` to an existing sink.
///
/// Keep the returned recording alive for the session and call `finish()` on
/// shutdown to flush it and report write errors. This does not install an
/// allocator profiler or periodically sample memory. An unset variable disables
/// recording; an empty path or a file creation failure is an error. Existing
/// files are never overwritten.
pub fn from_env(sink: &Sink) -> io::Result<Option<Recording<io::BufWriter<std::fs::File>>>> {
    let Some(path) = std::env::var_os("GUINEA_TRACE_FILE") else {
        return Ok(None);
    };
    let file = std::fs::File::create_new(path)?;
    Recording::start(sink, io::BufWriter::new(file)).map(Some)
}

/// A recording of one application's sink. Only that sink is subscribed to.
/// The writer must be `Send` because background threads write their own points.
pub struct Recording<W: Write + Send + 'static> {
    contexts: crate::sink::ContextStreaming,
    subscription: Streaming,
    state: Arc<Mutex<Writer<W>>>,
}

impl<W: Write + Send + 'static> Recording<W> {
    pub fn start(sink: &Sink, writer: W) -> io::Result<Self> {
        instrumentation(|| Self::start_inner(sink, writer))
    }

    fn start_inner(sink: &Sink, writer: W) -> io::Result<Self> {
        let mut state = Writer {
            writer: Some(writer),
            error: None,
            next_track: 2,
            execution: 0,
            threads: HashMap::new(),
            source_paths: HashMap::new(),
            lanes: HashMap::new(),
            open: HashMap::new(),
            parents: HashMap::new(),
            order: VecDeque::new(),
            counters: HashMap::new(),
            #[cfg(feature = "allocations")]
            memory_sample: 0,
            #[cfg(feature = "allocations")]
            memory_declarations: HashMap::new(),
        };
        let mut process = ProcessDescriptor::new();
        process.pid = Some(std::process::id() as i32);
        process.process_name = Some("Guinea".into());
        let mut descriptor = TrackDescriptor::new();
        descriptor.uuid = Some(1);
        descriptor.process = Some(process).into();
        // Map the private monotonic origin to the wall-clock origin already
        // captured by guinea-trace. This is not the OS monotonic/boot clock.
        let mut snapshot = ClockSnapshot::new();
        snapshot.primary_trace_clock = Some(protobuf::EnumOrUnknown::from_i32(TRACE_CLOCK as i32));
        let mut monotonic = Clock::new();
        monotonic.clock_id = Some(TRACE_CLOCK);
        monotonic.timestamp = Some(0);
        let mut realtime = Clock::new();
        realtime.clock_id = Some(1);
        realtime.timestamp = Some(ns(crate::started_at()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?));
        snapshot.clocks = vec![monotonic, realtime];
        let mut packet = TracePacket::new();
        packet.set_trusted_packet_sequence_id(1);
        packet.set_clock_snapshot(snapshot);
        state.packet(packet)?;
        state.descriptor(descriptor)?;
        #[cfg(windows)]
        state.attribute(
            "rs.guinea.process.creation",
            &process_creation_ns()?.to_string(),
        )?;
        let state = Arc::new(Mutex::new(state));
        let context_state = state.clone();
        let contexts = sink.stream_context(move |thread, at, cause| {
            let mut writer = context_state.lock().unwrap_or_else(PoisonError::into_inner);
            if writer.writer.is_some() && writer.error.is_none() {
                if let Err(error) = writer.thread(thread) {
                    writer.error = Some(error);
                    return;
                }
                let key = format!("rs.guinea.execution.{}", writer.execution);
                writer.execution += 1;
                let value = serde_json::json!({
                    "ts": ns(at), "tid": thread, "cause": cause.map(Cause::get),
                    "native_id": crate::sink::native_thread_id(),
                });
                if let Err(error) = writer.attribute(&key, &value.to_string()) {
                    writer.error = Some(error);
                }
            }
        });
        let shared = state.clone();
        let subscription = sink.stream(move |thread, at, trace| {
            let mut writer = shared.lock().unwrap_or_else(PoisonError::into_inner);
            if writer.writer.is_some() && writer.error.is_none() {
                if let Err(error) = writer.record(thread, at, trace) {
                    writer.error = Some(error);
                }
            }
        });
        Ok(Self {
            contexts,
            subscription,
            state,
        })
    }

    /// An absolute counter sample on Guinea's monotonic clock (for example
    /// `crate::now()`). Use a stable, bounded set of names; tracks live until finish.
    /// Missing measurements should be omitted, never replaced with zero.
    pub fn counter(&self, name: &str, at: Duration, value: f64) -> io::Result<()> {
        instrumentation(|| self.counter_inner(name, at, value))
    }

    fn counter_inner(&self, name: &str, at: Duration, value: f64) -> io::Result<()> {
        if !value.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "counter must be finite",
            ));
        }
        let mut writer = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(error) = &writer.error {
            return Err(io::Error::new(error.kind(), error.to_string()));
        }
        let result = writer.counter(name, at, value);
        if let Err(error) = &result {
            writer.error = Some(io::Error::new(error.kind(), error.to_string()));
        }
        result
    }

    /// Unsubscribes, waits for any current write and flushes. Returns the writer,
    /// or the first write error. Does not invent ends for unfinished operations.
    pub fn finish(self) -> io::Result<W> {
        instrumentation(|| self.finish_inner())
    }

    fn finish_inner(self) -> io::Result<W> {
        drop(self.contexts);
        drop(self.subscription);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut writer = state.writer.take().expect("recording finishes once");
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        writer.flush()?;
        Ok(writer)
    }

    /// Samples three aggregate Rust counters. No per-scope timeline tracks are
    /// created. Scope samples are trace attributes, queryable by the SQL
    /// package in `sql/guinea`. Sample before a workload and periodically after
    /// it; rates require at least two samples. Use one sampler per recording.
    /// Tracing is a subset of the Rust heap, not an additional amount to add.
    #[cfg(feature = "allocations")]
    pub fn memory(&self, profiler: &crate::allocations::Profiler) -> io::Result<()> {
        instrumentation(|| {
            let heap = crate::allocations::heap();
            let scopes = profiler.snapshot();
            let scoped: u64 = scopes.iter().map(|stats| stats.live_bytes).sum();
            let tracing = profiler.tracing();
            let at = crate::now();
            let mut writer = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(error) = &writer.error {
                return Err(io::Error::new(error.kind(), error.to_string()));
            }
            let result = (|| {
                writer.counter("Rust heap / live bytes", at, heap.live_bytes as f64)?;
                writer.counter("Guinea scopes / live bytes", at, scoped as f64)?;
                writer.counter(
                    "Tracing / attributed live bytes",
                    at,
                    tracing.live_bytes as f64,
                )?;
                writer.scope_samples(&scopes, at)
            })();
            if let Err(error) = &result {
                writer.error = Some(io::Error::new(error.kind(), error.to_string()));
            }
            result
        })
    }
}

#[cfg(windows)]
fn process_creation_ns() -> io::Result<u64> {
    use windows_sys::{
        Win32::Foundation::FILETIME,
        Win32::System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // Valid pseudo-handle and writable FILETIME outputs.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    Ok((ticks - 116444736000000000) * 100)
}

// TracePacket.trace_attributes (126) is newer than the generated schema.
// TraceAttributes.attribute = 1; Attribute.key = 1, string_value = 3.
fn attribute_packet(key: &str, value: &str) -> io::Result<TracePacket> {
    let encode = || -> protobuf::Result<Vec<u8>> {
        let mut attribute = Vec::new();
        {
            let mut output = protobuf::CodedOutputStream::vec(&mut attribute);
            output.write_string(1, key)?;
            output.write_string(3, value)?;
            output.flush()?;
        }
        let mut attributes = Vec::new();
        {
            let mut output = protobuf::CodedOutputStream::vec(&mut attributes);
            output.write_bytes(1, &attribute)?;
            output.flush()?;
        }
        Ok(attributes)
    };
    let mut packet = TracePacket::new();
    packet
        .mut_unknown_fields()
        .add_length_delimited(126, encode().map_err(io::Error::other)?);
    Ok(packet)
}

fn instrumentation<R>(work: impl FnOnce() -> R) -> R {
    #[cfg(feature = "allocations")]
    {
        crate::allocations::observing(work)
    }
    #[cfg(not(feature = "allocations"))]
    {
        work()
    }
}

fn native_thread_name() -> Option<String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::LocalFree,
            System::Threading::{GetCurrentThread, GetThreadDescription},
        };
        let mut pointer = std::ptr::null_mut();
        if GetThreadDescription(GetCurrentThread(), &mut pointer) < 0 || pointer.is_null() {
            return None;
        }
        let mut length = 0;
        while *pointer.add(length) != 0 {
            length += 1;
        }
        let name = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length));
        LocalFree(pointer.cast());
        return (!name.is_empty()).then_some(name);
    }
    #[cfg(not(windows))]
    None
}

#[derive(Clone, Copy)]
struct Anchor {
    track: u64,
    at: Duration,
}

struct Writer<W> {
    writer: Option<W>,
    error: Option<io::Error>,
    next_track: u64,
    execution: u64,
    threads: HashMap<u32, (std::thread::ThreadId, u64, String)>,
    source_paths: HashMap<(&'static str, Option<&'static str>), String>,
    lanes: HashMap<u32, Vec<(u64, bool)>>,
    open: HashMap<Cause, Anchor>,
    parents: HashMap<Cause, Anchor>,
    order: VecDeque<Cause>,
    counters: HashMap<String, u64>,
    #[cfg(feature = "allocations")]
    memory_sample: u64,
    #[cfg(feature = "allocations")]
    memory_declarations:
        HashMap<(u64, &'static str, &'static str), crate::allocations::Declaration>,
}

fn ns(at: Duration) -> u64 {
    at.as_nanos().min(u64::MAX as u128) as u64
}

fn uint(name: &str, value: u64) -> DebugAnnotation {
    let mut arg = DebugAnnotation::new();
    arg.set_name(name.into());
    arg.set_uint_value(value);
    arg
}

fn text(name: &str, value: String) -> DebugAnnotation {
    let mut arg = DebugAnnotation::new();
    arg.set_name(name.into());
    arg.set_string_value(value);
    arg
}

impl<W: Write> Writer<W> {
    // Stream callbacks run synchronously on the emitting thread, so its Rust
    // identity can be paired with the native id used by the stack sampler.
    fn thread(&mut self, tid: u32) -> io::Result<u64> {
        let current = std::thread::current();
        if let Some((id, track, _)) = self.threads.get(&tid) {
            if *id == current.id() {
                return Ok(*track);
            }
        }
        let rust_id = format!("{:?}", current.id());
        let name = current.name();
        let os_name = native_thread_name();
        let main = name == Some("main");
        let native_id = crate::sink::native_thread_id();
        let id_kind = if native_id.is_some() { "OS" } else { "Trace" };
        let label = format!(
            "{} · {rust_id} · {id_kind} {tid}",
            if main {
                "Main thread"
            } else {
                name.or(os_name.as_deref()).unwrap_or("Unnamed thread")
            }
        );
        let track = self.next_track;
        self.next_track += 1;
        let mut thread = ThreadDescriptor::new();
        thread.pid = Some(std::process::id() as i32);
        thread.tid = Some(tid as i32);
        thread.thread_name = Some(label.clone());
        let mut descriptor = TrackDescriptor::new();
        descriptor.uuid = Some(track);
        descriptor.parent_uuid = Some(1);
        descriptor.thread = Some(thread).into();
        self.descriptor(descriptor)?;
        self.attribute(
            &format!("rs.guinea.thread.{tid}.{rust_id}"),
            &serde_json::json!({
                "tid": tid, "rust_id": rust_id, "name": name, "os_name": os_name,
                "is_main": main, "native_id": native_id,
            })
            .to_string(),
        )?;
        // OS ids may be reused after a thread exits; Rust ids are not reused.
        self.lanes.remove(&tid);
        self.threads.insert(tid, (current.id(), track, label));
        Ok(track)
    }

    #[cfg(feature = "allocations")]
    fn scope_samples(
        &mut self,
        scopes: &[crate::allocations::Stats],
        at: Duration,
    ) -> io::Result<()> {
        for stats in scopes {
            for declaration in &stats.declarations {
                let key = (stats.id, declaration.kind, declaration.name);
                if self.memory_declarations.get(&key) == Some(declaration) {
                    continue;
                }
                let value = serde_json::json!({
                    "group_id": stats.id, "kind": declaration.kind, "name": declaration.name,
                    "file": declaration.file, "line": declaration.line, "column": declaration.column,
                    "crate_dir": declaration.crate_dir,
                });
                self.attribute(
                    &format!(
                        "rs.guinea.memory.declaration.{}.{}.{}",
                        stats.id, declaration.kind, declaration.name
                    ),
                    &value.to_string(),
                )?;
                self.memory_declarations.insert(key, declaration.clone());
            }
        }
        let samples: Vec<_> = scopes
            .iter()
            .map(|stats| {
                serde_json::json!({
                    "group_id": stats.id, "scope_id": stats.scope, "parent_group_id": stats.parent,
                    "closed": stats.closed, "closed_at_ns": stats.closed_at.map(ns),
                    "allocated_bytes": stats.allocated_bytes, "freed_bytes": stats.freed_bytes,
                    "live_bytes": stats.live_bytes, "peak_bytes": stats.peak_bytes,
                })
            })
            .collect();
        let batch = self.memory_sample;
        let value = serde_json::json!({"schema_version": 1, "ts": ns(at), "batch": batch, "scopes": samples});
        // Trace attributes replace previous values for a key. A unique key per
        // batch preserves history while keeping it off the timeline entirely.
        self.attribute(
            &format!("rs.guinea.memory.sample.{batch}"),
            &value.to_string(),
        )?;
        self.memory_sample += 1;
        Ok(())
    }

    fn attribute(&mut self, key: &str, value: &str) -> io::Result<()> {
        self.packet(attribute_packet(key, value)?)
    }
    fn packet(&mut self, mut packet: TracePacket) -> io::Result<()> {
        packet.set_trusted_packet_sequence_id(1);
        packet.timestamp_clock_id = Some(TRACE_CLOCK);
        packet.timestamp.get_or_insert(0);
        // Concatenated Trace messages append their repeated `packet` field.
        // Let the protobuf library encode the envelope as well as the packet.
        let mut trace = ProtoTrace::new();
        trace.packet.push(packet);
        trace
            .write_to_writer(self.writer.as_mut().expect("writer is attached"))
            .map_err(io::Error::other)
    }

    fn descriptor(&mut self, descriptor: TrackDescriptor) -> io::Result<()> {
        let mut packet = TracePacket::new();
        packet.set_track_descriptor(descriptor);
        self.packet(packet)
    }

    fn track(&mut self, name: String, counter: bool) -> io::Result<u64> {
        let track = self.next_track;
        self.next_track += 1;
        let mut descriptor = TrackDescriptor::new();
        descriptor.uuid = Some(track);
        descriptor.parent_uuid = Some(1);
        descriptor.set_name(name);
        if counter {
            descriptor.counter = Some(CounterDescriptor::new()).into();
        }
        self.descriptor(descriptor)?;
        Ok(track)
    }

    fn event(&mut self, at: Duration, event: TrackEvent) -> io::Result<()> {
        let mut packet = TracePacket::new();
        packet.timestamp = Some(ns(at));
        // Private, process-relative clock: do not pretend it is system BOOTTIME.
        packet.timestamp_clock_id = Some(TRACE_CLOCK);
        packet.set_trusted_packet_sequence_id(1);
        packet.set_track_event(event);
        self.packet(packet)
    }

    fn base(track: u64, kind: Type, name: &str) -> TrackEvent {
        let mut event = TrackEvent::new();
        event.track_uuid = Some(track);
        event.type_ = Some(kind.into());
        event.set_name(name.into());
        event.categories.push("guinea".into());
        event
    }

    fn lane(&mut self, thread: u32, busy: bool) -> io::Result<u64> {
        let parent = self.thread(thread)?;
        if let Some(lane) = self
            .lanes
            .entry(thread)
            .or_default()
            .iter_mut()
            .find(|lane| !lane.1)
        {
            lane.1 = busy;
            return Ok(lane.0);
        }
        let index = self.lanes[&thread].len();
        let track = self.next_track;
        self.next_track += 1;
        let mut descriptor = TrackDescriptor::new();
        descriptor.uuid = Some(track);
        descriptor.parent_uuid = Some(parent);
        descriptor.set_name(format!("{} / lane {index}", self.threads[&thread].2));
        self.descriptor(descriptor)?;
        self.lanes.get_mut(&thread).unwrap().push((track, busy));
        Ok(track)
    }

    fn record(&mut self, thread: u32, at: Duration, trace: &Trace) -> io::Result<()> {
        if let Trace::Mark(Record {
            point:
                Point::Plugin {
                    kind: "state.payload",
                    fields,
                },
            ..
        }) = trace
        {
            let get = |key| {
                fields
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value)
            };
            let id = get("update_id").ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "state payload has no id")
            })?;
            let payload = get("json").ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "state payload has no body")
            })?;
            return self.attribute(&format!("rs.guinea.state.{id}"), payload);
        }
        match trace {
            Trace::Begin(record) | Trace::Mark(record) => {
                let begin = matches!(trace, Trace::Begin(_));
                let track = self.lane(thread, begin)?;
                let name = match &record.point {
                    Point::Action { message } => Cow::Owned(format!("action · {message}")),
                    Point::Handle { actor, message } => Cow::Owned(format!("{message} → {actor}")),
                    Point::Send { actor, message } => {
                        Cow::Owned(format!("send · {message} → {actor}"))
                    }
                    Point::Publish { event, .. } => Cow::Owned(format!("publish · {event}")),
                    Point::Deliver { event } => Cow::Owned(format!("deliver · {event}")),
                    Point::Push { reducer } => Cow::Owned(format!("push · {reducer}")),
                    Point::Source {
                        source, to, output, ..
                    }
                    | Point::Pull { source, to, output }
                    | Point::Arrived {
                        source, to, output, ..
                    }
                    | Point::Closed {
                        source, to, output, ..
                    } => Cow::Owned(format!(
                        "{} · {output} → {to} · source #{source}",
                        record.point.kind()
                    )),
                    Point::Spawn { actor, output, .. }
                    | Point::Settled { actor, output, .. }
                    | Point::Cancelled { actor, output, .. }
                    | Point::Superseded { actor, output, .. }
                    | Point::Skipped { actor, output, .. } => {
                        Cow::Owned(format!("{} · {output} → {actor}", record.point.kind()))
                    }
                    Point::Render { segment, .. } => Cow::Owned(format!("render · {segment}")),
                    Point::Navigate { root, to } => Cow::Owned(format!("navigate · {root} → {to}")),
                    Point::Span { name, .. } => Cow::Borrowed(*name),
                    Point::Plugin {
                        kind: "state.update",
                        fields,
                    } => {
                        let reducer = fields
                            .iter()
                            .find(|(key, _)| *key == "reducer")
                            .map(|(_, value)| value.as_str())
                            .unwrap_or("state");
                        Cow::Owned(format!("state · {reducer}"))
                    }
                    Point::Plugin { kind, .. } => Cow::Borrowed(*kind),
                    Point::Log { target, text, .. } => Cow::Owned(format!("{target} · {text}")),
                };
                let mut event = Self::base(
                    track,
                    if begin {
                        Type::TYPE_SLICE_BEGIN
                    } else {
                        Type::TYPE_INSTANT
                    },
                    &name,
                );
                self.args(&mut event, record, thread);
                if let Some(parent) = record
                    .parent
                    .and_then(|parent| self.parents.get(&parent).copied())
                {
                    // A distinct flow per edge preserves fan-out. The origin is
                    // written retrospectively at its original timestamp.
                    let mut origin = Self::base(parent.track, Type::TYPE_INSTANT, "cause");
                    origin.flow_ids.push(record.id.get());
                    self.event(parent.at, origin)?;
                    event.terminating_flow_ids.push(record.id.get());
                }
                self.event(record.at, event)?;
                let anchor = Anchor {
                    track,
                    at: record.at,
                };
                if begin {
                    self.open.insert(record.id, anchor);
                }
                self.parents.insert(record.id, anchor);
                self.order.push_back(record.id);
                if self.order.len() > PARENTS {
                    self.parents.remove(&self.order.pop_front().unwrap());
                }
            }
            Trace::End { id, took } => {
                if let Some(anchor) = self.open.remove(id) {
                    let mut event = Self::base(anchor.track, Type::TYPE_SLICE_END, "");
                    event.clear_name();
                    event
                        .debug_annotations
                        .push(uint("guinea_took_ns", ns(*took)));
                    event
                        .debug_annotations
                        .push(uint("guinea_end_thread", thread as u64));
                    #[cfg(feature = "allocations")]
                    if let Some(stats) = crate::allocations::of_cause(*id) {
                        for (name, value) in [
                            ("memory_allocations", stats.allocations),
                            ("memory_allocated_bytes", stats.allocated_bytes),
                            ("memory_freed_bytes", stats.freed_bytes),
                            ("memory_live_bytes_at_end", stats.live_bytes),
                            ("memory_peak_bytes", stats.peak_bytes),
                        ] {
                            event.debug_annotations.push(uint(name, value));
                        }
                    }
                    self.event(at, event)?;
                    for lanes in self.lanes.values_mut() {
                        if let Some(lane) = lanes.iter_mut().find(|lane| lane.0 == anchor.track) {
                            lane.1 = false;
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn args(&mut self, event: &mut TrackEvent, record: &Record, thread: u32) {
        let mut location = record.location;
        match &record.point {
            Point::Source { file, line, .. } => {
                location.file = file;
                location.line = *line;
                location.column = 0;
                location.crate_dir = None;
            }
            Point::Log {
                file: Some(file),
                line,
                ..
            }
            | Point::Span {
                file: Some(file),
                line,
                ..
            } => {
                location.file = file;
                location.line = line.unwrap_or(0);
                location.column = 0;
                location.crate_dir = None;
            }
            _ => {}
        }
        let path = self
            .source_paths
            .entry((location.file, location.crate_dir))
            .or_insert_with(|| {
                let file = std::path::Path::new(location.file);
                if file.is_absolute() {
                    return location.file.to_owned();
                }
                let base = location
                    .crate_dir
                    .map(std::path::PathBuf::from)
                    .or_else(|| std::env::current_dir().ok());
                base.and_then(|base| {
                    base.ancestors()
                        .map(|dir| dir.join(file))
                        .find(|path| path.is_file())
                })
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| location.file.to_owned())
            });
        event
            .debug_annotations
            .push(text("file", location.file.into()));
        event
            .debug_annotations
            .push(text("source_path", path.clone()));
        if location.line > 0 {
            event
                .debug_annotations
                .push(uint("line", location.line as u64));
        }
        if location.column > 0 {
            event
                .debug_annotations
                .push(uint("column", location.column as u64));
        }
        if let Some(dir) = location.crate_dir {
            event.debug_annotations.push(text("crate_dir", dir.into()));
        }
        event
            .debug_annotations
            .push(uint("guinea_id", record.id.get()));
        event
            .debug_annotations
            .push(uint("guinea_thread", thread as u64));
        if let Some(parent) = record.parent {
            event
                .debug_annotations
                .push(uint("guinea_parent", parent.get()));
        }
        event
            .debug_annotations
            .push(text("guinea_description", record.point.to_string()));
        event
            .debug_annotations
            .push(text("guinea_kind", record.point.kind().into()));
        match &record.point {
            Point::Action { message } => event
                .debug_annotations
                .push(text("message", (*message).into())),
            Point::Send { actor, message } | Point::Handle { actor, message } => {
                event.debug_annotations.push(text("actor", (*actor).into()));
                event
                    .debug_annotations
                    .push(text("message", (*message).into()));
            }
            Point::Push { reducer } => event
                .debug_annotations
                .push(text("reducer", (*reducer).into())),
            Point::Publish {
                event: name,
                subscribers,
            } => {
                event.debug_annotations.push(text("event", (*name).into()));
                event
                    .debug_annotations
                    .push(uint("subscribers", *subscribers as u64));
            }
            Point::Deliver { event: name } => {
                event.debug_annotations.push(text("event", (*name).into()))
            }
            Point::Source {
                source, to, output, ..
            }
            | Point::Pull { source, to, output }
            | Point::Arrived {
                source, to, output, ..
            }
            | Point::Closed {
                source, to, output, ..
            } => {
                event.debug_annotations.push(uint("source", *source));
                event.debug_annotations.push(text("to", (*to).into()));
                event
                    .debug_annotations
                    .push(text("output", (*output).into()));
                if let Point::Arrived {
                    name: Some(name), ..
                } = &record.point
                {
                    event.debug_annotations.push(text("name", (*name).into()));
                }
            }
            Point::Render { segment, .. } => event
                .debug_annotations
                .push(text("segment", (*segment).into())),
            Point::Spawn {
                actor,
                actor_id,
                output,
            }
            | Point::Skipped {
                actor,
                actor_id,
                output,
            }
            | Point::Settled {
                actor,
                actor_id,
                output,
                ..
            }
            | Point::Cancelled {
                actor,
                actor_id,
                output,
                ..
            }
            | Point::Superseded {
                actor,
                actor_id,
                output,
                ..
            } => {
                event.debug_annotations.push(text("actor", (*actor).into()));
                event.debug_annotations.push(uint("actor_id", *actor_id));
                event
                    .debug_annotations
                    .push(text("output", (*output).into()));
            }
            Point::Navigate { root, to } => {
                event.debug_annotations.push(text("root", root.clone()));
                event.debug_annotations.push(text("to", to.clone()));
            }
            Point::Log {
                level,
                target,
                text: message,
                ..
            } => {
                event
                    .debug_annotations
                    .push(text("level", level.to_string()));
                event
                    .debug_annotations
                    .push(text("target", (*target).into()));
                event
                    .debug_annotations
                    .push(text("message", message.clone()));
            }
            Point::Span {
                name,
                target,
                fields,
                ..
            } => {
                event.debug_annotations.push(text("name", (*name).into()));
                event
                    .debug_annotations
                    .push(text("target", (*target).into()));
                event.debug_annotations.push(text("fields", fields.clone()));
            }
            _ => {}
        }
        match &record.point {
            Point::Settled { took_us, .. }
            | Point::Cancelled { took_us, .. }
            | Point::Superseded { took_us, .. }
            | Point::Closed { took_us, .. }
            | Point::Render { took_us, .. } => {
                event.debug_annotations.push(uint("took_us", *took_us))
            }
            _ => {}
        }
        if let Point::Plugin { fields, .. } = &record.point {
            for (name, value) in fields {
                event.debug_annotations.push(text(name, value.clone()));
            }
        }
    }

    fn counter(&mut self, name: &str, at: Duration, value: f64) -> io::Result<()> {
        let track = if let Some(track) = self.counters.get(name) {
            *track
        } else {
            let track = self.track(name.into(), true)?;
            self.counters.insert(name.into(), track);
            track
        };
        let mut event = Self::base(track, Type::TYPE_COUNTER, name);
        event.set_double_counter_value(value);
        self.event(at, event)
    }
}
