//! Post-process sampled stacks; causal frames are explicitly synthetic.
use std::collections::{HashMap, HashSet};
use std::io;

use perfetto_protos::{
    interned_data::InternedData,
    profile_common::{Callstack, Frame, InternedString, Mapping},
    profile_packet::PerfSample,
    trace::Trace,
    trace_packet::TracePacket,
};
use protobuf::{Message, UnknownValueRef};
use serde_json::Value;

/// Merge a processed samply profile with a Guinea trace into native Perfetto.
///
/// Semantic frames start at the first resolved Guinea frame, after native
/// callers. Guinea wrapper frames are collapsed; application frames stay in
/// order. Unresolved boundaries are left untouched. Context
/// attribution uses OS thread ids and recorded execution transitions, not the
/// lifetime of an async task. Unmatched samples remain unassociated. The samply
/// symbol sidecar is optional; unresolved addresses remain explicit.
/// Native identity comes from the recording, never the merger's host platform.
/// Internal or unknown thread identities (including macOS recordings) preserve
/// native stacks without causal attribution. Legacy recordings can supply
/// native identity through their `rs.guinea.thread.*` metadata.
pub fn merge_cpu(profile: &Value, symbols: Option<&Value>, guinea: &[u8]) -> io::Result<Vec<u8>> {
    merge(profile, symbols, guinea, StackView::Guinea).map_err(io::Error::other)
}

/// Preserve sampled stacks exactly, without aliases or synthetic ancestors.
pub fn merge_cpu_raw(
    profile: &Value,
    symbols: Option<&Value>,
    guinea: &[u8],
) -> io::Result<Vec<u8>> {
    merge(profile, symbols, guinea, StackView::Raw).map_err(io::Error::other)
}

#[derive(Clone, Copy, PartialEq)]
enum StackView {
    Guinea,
    Raw,
}

#[derive(Clone, Debug, PartialEq)]
struct NativeFrame {
    name: String,
    module: String,
    address: u64,
}

fn is_guinea_frame(name: &str) -> bool {
    let owner = name.trim_start_matches('<');
    [
        "guinea::",
        "guinea_core::",
        "guinea_app::",
        "guinea_router::",
        "guinea_trace::",
        "guinea_winui::",
        "guinea_eframe::",
        "guinea_iced::",
        "guinea_slint::",
        "guinea_ratatui::",
    ]
    .iter()
    .any(|prefix| owner.starts_with(prefix))
}

fn project_stack(native: Vec<NativeFrame>, labels: &[String], view: StackView) -> Vec<NativeFrame> {
    if view == StackView::Raw || labels.is_empty() {
        return native;
    }
    let Some(boundary) = native.iter().position(|frame| is_guinea_frame(&frame.name)) else {
        return native;
    };
    let mut projected = native[..boundary].to_vec();
    projected.extend(labels.iter().map(|name| NativeFrame {
        name: name.clone(),
        module: "Guinea causal context".into(),
        address: 0,
    }));
    projected.extend(
        native
            .into_iter()
            .skip(boundary)
            .filter(|frame| !is_guinea_frame(&frame.name)),
    );
    projected
}

fn merge(
    profile: &Value,
    symbols: Option<&Value>,
    guinea: &[u8],
    view: StackView,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut trace = Trace::parse_from_bytes(guinea)?;
    let origin = trace
        .packet
        .iter()
        .filter(|p| p.has_clock_snapshot())
        .flat_map(|p| &p.clock_snapshot().clocks)
        .find(|c| c.clock_id() == 1)
        .ok_or("Guinea trace has no realtime clock anchor")?
        .timestamp();
    let app_pid = trace
        .packet
        .iter()
        .filter(|p| p.has_track_descriptor())
        .find_map(|p| {
            p.track_descriptor()
                .process
                .as_ref()
                .map(|p| p.pid() as u32)
        })
        .ok_or("Guinea trace has no process descriptor")?;
    let mut causes = HashMap::<u64, (Option<u64>, String)>::new();
    let mut contexts = HashMap::<u32, Vec<(u64, Option<u64>)>>::new();
    let mut executions = Vec::<Value>::new();
    let mut native_threads = HashMap::<u32, Option<u32>>::new();
    let mut creation = None;
    for packet in &trace.packet {
        if packet.has_track_event() {
            let event = packet.track_event();
            let number = |key| {
                event
                    .debug_annotations
                    .iter()
                    .find(|a| a.name() == key)
                    .filter(|a| a.has_uint_value())
                    .map(|a| a.uint_value())
            };
            let description = event
                .debug_annotations
                .iter()
                .find(|a| a.name() == "guinea_description")
                .map(|a| a.string_value().to_owned());
            if let (Some(id), Some(description)) = (number("guinea_id"), description) {
                causes.insert(id, (number("guinea_parent"), description));
            }
        }
        if let Some(UnknownValueRef::LengthDelimited(bytes)) =
            packet.special_fields.unknown_fields().get(126)
        {
            for (_, attribute) in wire_strings(bytes)? {
                let fields = wire_strings(&attribute)?;
                let key = fields
                    .iter()
                    .find(|(n, _)| *n == 1)
                    .map(|(_, v)| String::from_utf8_lossy(v));
                if key
                    .as_ref()
                    .is_some_and(|key| key.starts_with("rs.guinea.execution."))
                {
                    let bytes = &fields
                        .iter()
                        .find(|(n, _)| *n == 3)
                        .ok_or("missing execution value")?
                        .1;
                    let value: Value = serde_json::from_slice(bytes)?;
                    executions.push(value);
                } else if key
                    .as_ref()
                    .is_some_and(|key| key.starts_with("rs.guinea.thread."))
                {
                    let bytes = &fields
                        .iter()
                        .find(|(n, _)| *n == 3)
                        .ok_or("missing thread identity value")?
                        .1;
                    let value: Value = serde_json::from_slice(bytes)?;
                    if let Some(tid) = identifier(&value["tid"]) {
                        let native = identifier(&value["native_id"]);
                        native_threads
                            .entry(tid)
                            .and_modify(|previous| {
                                // Conflicting provenance must never authorize a match.
                                if *previous != native {
                                    *previous = None;
                                }
                            })
                            .or_insert(native);
                    }
                } else if key.as_deref() == Some("rs.guinea.process.creation") {
                    let bytes = &fields
                        .iter()
                        .find(|(n, _)| *n == 3)
                        .ok_or("missing creation value")?
                        .1;
                    creation = Some(serde_json::from_slice::<u64>(bytes)?);
                }
            }
        }
    }
    for execution in executions {
        let tid = identifier(&execution["tid"]).ok_or("invalid execution tid")?;
        let at = execution["ts"].as_u64().ok_or("invalid execution time")?;
        let native = match execution.get("native_id") {
            // Explicit null/invalid provenance overrides legacy thread metadata.
            Some(value) => identifier(value),
            None => native_threads.get(&tid).copied().flatten(),
        };
        if let Some(native) = native {
            contexts
                .entry(native)
                .or_default()
                .push((at, execution["cause"].as_u64()));
        }
    }
    for context in contexts.values_mut() {
        context.sort_by_key(|entry| entry.0);
    }
    let start = profile["meta"]["startTime"]
        .as_f64()
        .ok_or("CPU profile has no startTime")?;
    let startup = profile["threads"]
        .as_array()
        .and_then(|threads| {
            threads
                .iter()
                .find(|thread| identifier(&thread["pid"]) == Some(app_pid))
        })
        .and_then(|thread| thread["processStartupTime"].as_f64());
    let cpu_origin = match creation.zip(startup) {
        Some((creation, startup)) => creation
            .checked_sub((startup * 1e6).round() as u64)
            .ok_or("invalid process clock anchor")?,
        None => (start * 1e6).round() as u64,
    };
    let mut intern = Intern::default();
    let mut packets = Vec::new();
    for thread in profile["threads"]
        .as_array()
        .ok_or("CPU profile has no threads")?
    {
        let pid = identifier(&thread["pid"]).ok_or("invalid CPU pid")?;
        let tid = identifier(&thread["tid"]).ok_or("invalid CPU tid")?;
        let samples = &thread["samples"];
        let mut time = 0.0;
        for (index, stack) in samples["stack"]
            .as_array()
            .ok_or("missing sample stacks")?
            .iter()
            .enumerate()
        {
            if let Some(times) = samples["time"].as_array() {
                time = times[index].as_f64().ok_or("invalid sample time")?;
            } else {
                time += samples["timeDeltas"][index]
                    .as_f64()
                    .ok_or("missing sample time delta")?;
            }
            let Some(mut stack) = stack.as_u64().map(|v| v as usize) else {
                continue;
            };
            // samply also records compressed off-CPU rows (e.g. a wait with
            // weight 74). They are useful in its timeline, but are not CPU work.
            if samples["threadCPUDelta"][index].as_f64() == Some(0.0) {
                continue;
            }
            let timestamp = cpu_origin + (time * 1e6).round() as u64;
            let cause = if pid == app_pid {
                timestamp.checked_sub(origin).and_then(|at| {
                    contexts.get(&tid).and_then(|entries| {
                        entries
                            .partition_point(|entry| entry.0 <= at)
                            .checked_sub(1)
                            .and_then(|i| entries[i].1)
                    })
                })
            } else {
                None
            };
            let mut labels = Vec::new();
            let mut seen = HashSet::new();
            let mut current = cause;
            while let Some(id) = current {
                if !seen.insert(id) {
                    return Err("cycle in Guinea causal graph".into());
                }
                let Some((parent, description)) = causes.get(&id) else {
                    break;
                };
                labels.push(format!("[Guinea] {description}"));
                current = *parent;
            }
            labels.reverse();
            let mut native = Vec::new();
            let mut seen_stacks = HashSet::new();
            loop {
                if !seen_stacks.insert(stack) {
                    return Err("cycle in sampled stack".into());
                }
                let frame = thread["stackTable"]["frame"][stack]
                    .as_u64()
                    .ok_or("invalid stack frame")? as usize;
                let (name, module, address) = native_frame(profile, thread, frame, symbols);
                native.push(NativeFrame {
                    name,
                    module,
                    address,
                });
                match thread["stackTable"]["prefix"][stack].as_u64() {
                    Some(prefix) => stack = prefix as usize,
                    None => break,
                }
            }
            native.reverse();
            let frames = project_stack(native, &labels, view)
                .into_iter()
                .map(|frame| intern.frame(&frame.name, &frame.module, frame.address))
                .collect();
            let callstack = intern.stack(frames);
            let mut sample = PerfSample::new();
            sample.pid = Some(pid);
            sample.tid = Some(tid);
            sample.callstack_iid = Some(callstack);
            let mut packet = TracePacket::new();
            packet.set_perf_sample(sample);
            packet.timestamp = Some(timestamp);
            packet.timestamp_clock_id = Some(1);
            packet.set_trusted_packet_sequence_id(2);
            packets.push(packet);
        }
    }
    let mut packet = TracePacket::new();
    // Keep startup samples (which precede the first Guinea event) non-negative.
    let shift = origin.saturating_sub(cpu_origin);
    if shift > 0 {
        shift_origin(&mut trace, shift)?;
    }
    packet.set_trusted_packet_sequence_id(2);
    packet.set_sequence_flags(1); // SEQ_INCREMENTAL_STATE_CLEARED
    packet.interned_data = Some(intern.data).into();
    trace.packet.push(packet);
    trace.packet.extend(packets);
    let interval = profile["meta"]["interval"]
        .as_f64()
        .filter(|interval| interval.is_finite() && *interval > 0.0)
        .ok_or("CPU profile has no positive sampling interval")?;
    trace.packet.push(super::attribute_packet(
        "rs.guinea.cpu.interval_ns",
        &((interval * 1e6).round() as u64).to_string(),
    )?);
    Ok(trace.write_to_bytes()?)
}

fn identifier(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

fn shift_origin(
    trace: &mut Trace,
    shift: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for packet in &mut trace.packet {
        if packet.has_clock_snapshot() {
            let snapshot = packet.mut_clock_snapshot();
            for clock in &mut snapshot.clocks {
                if clock.clock_id() == 11 {
                    clock.timestamp = Some(clock.timestamp() + shift);
                }
            }
        }
        if packet.timestamp_clock_id() == 11 {
            packet.timestamp = packet.timestamp.map(|ts| ts + shift);
        }
        let Some(UnknownValueRef::LengthDelimited(bytes)) =
            packet.special_fields.unknown_fields().get(126)
        else {
            continue;
        };
        let mut output = Vec::new();
        for (field, attribute) in wire_strings(bytes)? {
            let mut encoded = Vec::new();
            for (number, mut bytes) in wire_strings(&attribute)? {
                if number == 3 {
                    if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) {
                        if let Some(ts) = value["ts"].as_u64() {
                            value["ts"] = (ts + shift).into();
                        }
                        if let Some(scopes) = value.get_mut("scopes").and_then(Value::as_array_mut)
                        {
                            for scope in scopes {
                                if let Some(ts) = scope["closed_at_ns"].as_u64() {
                                    scope["closed_at_ns"] = (ts + shift).into();
                                }
                            }
                        }
                        bytes = serde_json::to_vec(&value)?;
                    }
                }
                let mut stream = protobuf::CodedOutputStream::vec(&mut encoded);
                stream.write_bytes(number, &bytes)?;
                stream.flush()?;
            }
            let mut stream = protobuf::CodedOutputStream::vec(&mut output);
            stream.write_bytes(field, &encoded)?;
            stream.flush()?;
        }
        packet.special_fields.mut_unknown_fields().remove(126);
        packet
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(126, output);
    }
    Ok(())
}

fn native_frame(
    profile: &Value,
    thread: &Value,
    frame: usize,
    symbols: Option<&Value>,
) -> (String, String, u64) {
    let function = thread["frameTable"]["func"][frame].as_u64().unwrap_or(0) as usize;
    let name_index = thread["funcTable"]["name"][function].as_u64().unwrap_or(0) as usize;
    let mut name = thread["stringArray"][name_index]
        .as_str()
        .unwrap_or("[unknown]")
        .to_owned();
    let address = thread["frameTable"]["address"][frame].as_u64().unwrap_or(0);
    let lib = thread["funcTable"]["resource"][function]
        .as_u64()
        .and_then(|resource| thread["resourceTable"]["lib"][resource as usize].as_u64())
        .map(|lib| &profile["libs"][lib as usize]);
    let module = lib
        .and_then(|lib| lib["name"].as_str())
        .unwrap_or("[native]")
        .to_owned();
    if let (Some(lib), Some(symbols)) = (lib, symbols) {
        if let Some(data) = symbols["data"].as_array().and_then(|data| {
            data.iter().find(|data| {
                data["debug_name"] == lib["debugName"] && data["code_id"] == lib["codeId"]
            })
        }) {
            if let Some(entry) = data["known_addresses"].as_array().and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry[0].as_u64() == Some(address))
            }) {
                if let Some(symbol) = entry[1]
                    .as_u64()
                    .and_then(|index| data["symbol_table"][index as usize]["symbol"].as_u64())
                    .and_then(|index| symbols["string_table"][index as usize].as_str())
                {
                    name = symbol.to_owned();
                }
            }
        }
    }
    (name, module, address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frames(names: &[&str]) -> Vec<NativeFrame> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| NativeFrame {
                name: (*name).into(),
                module: "app.exe".into(),
                address: index as u64 + 100,
            })
            .collect()
    }

    #[test]
    fn cpu_identity_uses_recorded_provenance_and_preserves_untrusted_stacks() {
        let sink = crate::Sink::new();
        let recording = super::super::Recording::start(&sink, Vec::new()).unwrap();
        let cause;
        {
            let _sink = crate::record_into(&sink);
            let action = crate::enter(|| crate::Point::Action { message: "Refresh" });
            cause = action.id().get();
        }
        let mut trace = Trace::parse_from_bytes(&recording.finish().unwrap()).unwrap();
        let mut execution_count = 0;
        for packet in &trace.packet {
            if let Some(UnknownValueRef::LengthDelimited(bytes)) =
                packet.special_fields.unknown_fields().get(126)
            {
                for (_, attribute) in wire_strings(bytes).unwrap() {
                    let fields = wire_strings(&attribute).unwrap();
                    let key = fields.iter().find(|(n, _)| *n == 1).unwrap();
                    if key.1.starts_with(b"rs.guinea.execution.") {
                        let body = fields.iter().find(|(n, _)| *n == 3).unwrap();
                        let value: Value = serde_json::from_slice(&body.1).unwrap();
                        assert_eq!(
                            value.get("native_id"),
                            Some(&json!(crate::sink::native_thread_id()))
                        );
                        execution_count += 1;
                    }
                }
            }
        }
        assert!(
            execution_count >= 2,
            "enter and exit export identity provenance"
        );
        // Replace host-generated identities with a foreign recording. This test
        // must give the same result on Windows, Linux and unsupported hosts.
        for packet in &mut trace.packet {
            packet.special_fields.mut_unknown_fields().remove(126);
        }
        let origin = trace
            .packet
            .iter()
            .filter(|p| p.has_clock_snapshot())
            .flat_map(|p| &p.clock_snapshot().clocks)
            .find(|clock| clock.clock_id() == 1)
            .unwrap()
            .timestamp();
        let profile = json!({"meta":{"startTime":origin as f64 / 1e6,"interval":10},"threads":[{
            "pid":std::process::id(),"tid":7,
            "samples":{"time":[100],"stack":[2]},
            "stackTable":{"frame":[0,1,2],"prefix":[null,0,1]},
            "frameTable":{"func":[0,1,2],"address":[11,22,33]},
            "funcTable":{"name":[0,1,2],"resource":[-1,-1,-1]},
            "stringArray":["app::main","guinea_core::executor::run","app::work"]
        }]});
        // None means a legacy execution without its own identity field. A
        // numeric native_id can differ from the trace-local tid.
        for (native, legacy, attributed) in [
            (Some(json!(null)), Some(json!(7)), false),
            (Some(json!("invalid")), Some(json!(7)), false),
            (Some(json!(u64::from(u32::MAX) + 1)), None, false),
            (None, None, false),
            (None, Some(json!(null)), false),
            (Some(json!(7)), None, true),
            (None, Some(json!(7)), true), // Legacy Windows provenance.
        ] {
            let mut recorded = trace.clone();
            // Internal IDs deliberately collide with samply's native TID.
            let tid = if attributed { 42 } else { 7 };
            let mut execution = json!({"ts":0,"tid":tid,"cause":cause});
            if let Some(native) = native {
                execution["native_id"] = native;
            }
            // Metadata order must not matter: legacy declarations follow use.
            recorded.packet.push(
                super::super::attribute_packet("rs.guinea.execution.0", &execution.to_string())
                    .unwrap(),
            );
            if let Some(native) = legacy {
                recorded.packet.push(
                    super::super::attribute_packet(
                        &format!("rs.guinea.thread.{tid}.ThreadId(1)"),
                        &json!({"tid":tid,"native_id":native}).to_string(),
                    )
                    .unwrap(),
                );
            }
            let bytes = recorded.write_to_bytes().unwrap();
            let merged =
                Trace::parse_from_bytes(&merge_cpu(&profile, None, &bytes).unwrap()).unwrap();
            let raw =
                Trace::parse_from_bytes(&merge_cpu_raw(&profile, None, &bytes).unwrap()).unwrap();
            let intern = |trace: &Trace| {
                trace
                    .packet
                    .iter()
                    .find_map(|p| p.interned_data.as_ref())
                    .unwrap()
                    .clone()
            };
            let projected = intern(&merged);
            assert_eq!(
                merged.packet.iter().filter(|p| p.has_perf_sample()).count(),
                1
            );
            if attributed {
                assert!(
                    projected
                        .function_names
                        .iter()
                        .any(|n| n.str().starts_with(b"[Guinea]"))
                );
            } else {
                assert_eq!(projected, intern(&raw));
                assert_eq!(
                    projected
                        .frames
                        .iter()
                        .map(|f| f.rel_pc())
                        .collect::<Vec<_>>(),
                    [11, 22, 33]
                );
            }
        }
    }

    #[test]
    fn public_method_renames_do_not_move_semantic_boundaries() {
        let labels = vec![
            "[Guinea] action Add".into(),
            "[Guinea] Editor handles Add".into(),
        ];
        let stack = frames(&[
            "std::rt::lang_start",
            "app::main",
            "app::click<guinea_core::Dispatch>",
            "guinea_core::feature::dispatch::Dispatch::emit<Add>",
            "guinea_core::actor::addr::Addr<Editor>::send<Add>",
            "guinea_core::actor::envelope::deliver<Editor, Add>",
            "guinea_trace::allocations::Group::run",
            "app::Editor::add",
            "app::commit",
        ]);
        let expected = project_stack(stack.clone(), &labels, StackView::Guinea);
        insta::assert_debug_snapshot!(
            "guinea_boundary",
            expected.iter().map(|frame| &frame.name).collect::<Vec<_>>()
        );
        let mut renamed = stack.clone();
        renamed[3].name = "guinea_core::feature::dispatch::Dispatch::totally_new_api<Add>".into();
        renamed[4].name = "<guinea_core::actor::addr::Addr<Editor>>::renamed_enqueue<Add>".into();
        renamed[5].name = "guinea_core::actor::envelope::another_private_name<Editor, Add>".into();
        renamed[6].name = "guinea_trace::allocations::different_helper".into();
        assert_eq!(project_stack(renamed, &labels, StackView::Guinea), expected);
        assert_eq!(project_stack(stack.clone(), &labels, StackView::Raw), stack);
    }

    #[test]
    fn no_guessed_boundary_for_unresolved_or_application_owned_frames() {
        let labels = vec!["[Guinea] action Add".into()];
        for stack in [
            frames(&["0x100", "0x200"]),
            frames(&[
                "app::main",
                "app::uses<guinea_core::Dispatch>",
                "guinea_core_helpers::work",
            ]),
        ] {
            assert_eq!(
                project_stack(stack.clone(), &labels, StackView::Guinea),
                stack
            );
        }
        let stack = frames(&["app::main", "guinea_core::executor::run", "app::work"]);
        assert_eq!(project_stack(stack.clone(), &[], StackView::Guinea), stack);
    }

    #[test]
    fn sidecar_resolves_only_addresses_from_the_matching_binary() {
        let profile = json!({"libs":[{"name":"demo.exe","debugName":"demo.pdb","codeId":"ABC"}]});
        let thread = json!({"frameTable":{"func":[0],"address":[64]},
            "funcTable":{"name":[0],"resource":[0]},"stringArray":["0x40"],"resourceTable":{"lib":[0]}});
        let mut symbols = json!({"string_table":["demo::environments_from_scan"],"data":[{
            "debug_name":"demo.pdb","code_id":"ABC","known_addresses":[[64,0]],"symbol_table":[{"symbol":0}]
        }]});
        assert_eq!(
            native_frame(&profile, &thread, 0, Some(&symbols)),
            ("demo::environments_from_scan".into(), "demo.exe".into(), 64)
        );
        symbols["data"][0]["code_id"] = "OTHER".into();
        assert_eq!(native_frame(&profile, &thread, 0, Some(&symbols)).0, "0x40");
    }
}

#[derive(Default)]
struct Intern {
    data: InternedData,
    frames: HashMap<(String, String, u64), u64>,
    stacks: HashMap<Vec<u64>, u64>,
    mappings: HashMap<String, u64>,
}

impl Intern {
    fn frame(&mut self, name: &str, module: &str, address: u64) -> u64 {
        let key = (name.to_owned(), module.to_owned(), address);
        if let Some(id) = self.frames.get(&key) {
            return *id;
        }
        let mapping = *self.mappings.entry(module.to_owned()).or_insert_with(|| {
            let id = self.data.mappings.len() as u64 + 1;
            let mut path = InternedString::new();
            path.iid = Some(id);
            path.str = Some(module.as_bytes().to_vec());
            self.data.mapping_paths.push(path);
            let mut mapping = Mapping::new();
            mapping.iid = Some(id);
            mapping.path_string_ids.push(id);
            self.data.mappings.push(mapping);
            id
        });
        let id = self.data.frames.len() as u64 + 1;
        let mut string = InternedString::new();
        string.iid = Some(id);
        string.str = Some(name.as_bytes().to_vec());
        self.data.function_names.push(string);
        let mut frame = Frame::new();
        frame.iid = Some(id);
        frame.function_name_id = Some(id);
        frame.mapping_id = Some(mapping);
        frame.rel_pc = Some(if module == "Guinea causal context" {
            id
        } else {
            address
        });
        self.data.frames.push(frame);
        self.frames.insert(key, id);
        id
    }
    fn stack(&mut self, frames: Vec<u64>) -> u64 {
        if let Some(id) = self.stacks.get(&frames) {
            return *id;
        }
        let id = self.data.callstacks.len() as u64 + 1;
        let mut stack = Callstack::new();
        stack.iid = Some(id);
        stack.frame_ids = frames.clone();
        self.data.callstacks.push(stack);
        self.stacks.insert(frames, id);
        id
    }
}

fn wire_strings(bytes: &[u8]) -> protobuf::Result<Vec<(u32, Vec<u8>)>> {
    let mut input = protobuf::CodedInputStream::from_bytes(bytes);
    let mut fields = Vec::new();
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        if tag & 7 == 2 {
            fields.push((tag >> 3, input.read_bytes()?));
        } else {
            protobuf::rt::read_unknown_or_skip_group(
                tag,
                &mut input,
                &mut protobuf::UnknownFields::new(),
            )?;
        }
    }
    Ok(fields)
}
