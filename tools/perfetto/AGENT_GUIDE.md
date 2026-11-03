# Profiling Guinea applications as an agent

Use this workflow to investigate CPU cost, allocations, causal events and reducer
changes. Run commands from the repository root. The implementation and build
requirements are in [README.md](README.md).

## Choose the recording

Install once with `cargo install --locked guinea-profile`.
The standalone CLI accepts an existing executable and does not build the app.
See [installation and portable UI assets](../guinea-profile/README.md). When
running directly from source, use `cargo run --locked -p guinea-profile -- ...`.


| Task | Command |
| --- | --- |
| Reference application, normal recording | `cargo xtask profile` |
| Small reference with state values and diffs | `cargo xtask profile -- 64 --state-details` |
| Record without starting the viewer | `cargo xtask profile --no-ui` |
| Serve without opening a browser | `cargo xtask profile --no-open` |
| Recover and symbolize saved inputs | `guinea-profile process <source-directory> --output <new-directory>` |
| Reopen a recording | `guinea-profile view <recording-directory>` |
| Profile another application | `cargo xtask profile --manifest-path ../app/Cargo.toml -p app --bin app` |
| Profile an existing executable | `guinea-profile record -- path/to/app.exe app-arguments` |

Arguments after `--` belong to the application. `64` and `--state-details` are
arguments of the reference todo binary, not universal Guinea or xtask options.
Use `guinea-profile record --help` for sample rate, duration and output options.
Cargo target and feature options belong to `cargo xtask profile --help`. An explicit `--output` must name a new
directory.

The reference runs real `todo-core` application logic and storage through the
harness. It does **not** render a GUI. Its results cannot demonstrate the cost of
WinUI layout, a SwapChain or another application's rendering. Profile the actual
application and reproduce its workload for those questions.

Use normal mode for the first CPU/memory investigation. Enable detailed state
capture explicitly when values are needed; serialization and diff generation
add work. Compare runs with the same workload, build settings, recording mode
and sampling rate.

## Run and preserve evidence

1. Build the application with symbols, then let guinea-profile record, resolve
   symbols and merge the trace. The xtask reference wrapper builds for you.
   Read failures before retrying. Partial recordings are preserved on failure.
2. Record the printed `Recording:` directory and `Perfetto:` URL. The viewer
   keeps running until stopped; an ongoing server process is not a failed command.
3. Use the printed URL, including its plugin parameter. Ports are ephemeral;
   never copy an old chat's port into a new result.
4. Reopen the existing recording for UI or SQL changes. Do not repeat application
   profiling merely to change a chart or panel.
5. Keep original inputs. Do not overwrite an existing recording to make a new
   comparison or rerun the merger over existing output files.

The standalone profiler prepares cached tools and the pinned Perfetto UI. Do not replace this with
an ad hoc server, dashboard imports, global environment settings or a different
profiler. Keep normal build profiles and compiler caches unchanged. On Windows,
the UI build requires WSL and CPU recording requires Windows Performance Toolkit.
If a prerequisite is missing, identify it from the error and follow the documented
setup; do not silently omit CPU recording or substitute another profiler.

Useful files in a completed recording:

| File | Contents |
| --- | --- |
| `session.json` | Target, arguments, sample rate and recording status |
| `cpu.json.gz` | Original samply recording |
| `cpu.json.syms.json` | Symbol sidecar, when available |
| `guinea.pftrace` | Guinea events, counters and recording metadata |
| `combined.pftrace` | CPU samples with the Guinea stack projection and events |
| `raw.pftrace` | The same sampled CPU data without aliases or synthetic ancestors |

Older recordings may lack newer files. Preserve them; absence is not permission
to invent missing data. Reopening needs `combined.pftrace` and does not rerun the
application. The viewer binds to loopback.

`record` and `process` automatically symbolize recorded addresses. For recovery,
use `process <source> --output <new-directory>`; never alter original IDs or hand
assemble a sidecar to compensate for an importer failure. Supply `--executable`
and repeatable `--symbol-dir` when binaries/PDBs have moved. Inspect
`symbolication.json` for module identity failures, timeouts and partial resolution.
Missing application symbols are errors unless `--allow-unresolved` was explicit.
Windows recording preserves ETL for sampler/event-source investigations. Keep
those originals too. Runtime UI builds now publish immutable distributions and
stage bundled sources directly into WSL, independently of Windows drive mounts.

## Inspect in Perfetto

The **Guinea** sidebar entry opens dashboards. The **Guinea workspace** contains
the timeline tracks. These are different views of the same recording.

| Question | Where to look |
| --- | --- |
| What happened and what caused it? | `Events · ...` timeline tracks; select an event and inspect its debug fields and causal links |
| Where is the code? | `source_path`, `file`, `line`, `column`; handlers use available declaration metadata |
| Which thread? | Thread name, Rust `ThreadId` and native OS ID on Windows/Linux |
| Where did CPU samples land over time? | CPU flamechart |
| What are the aggregate sampled stacks? | Top search: enter `>`, choose `Select all perf samples`, then open `Callstack Flamegraph` in the bottom panel |
| What about a particular interval? | Select an area on CPU tracks to aggregate that interval |
| Which operations allocate? | `Events` dashboard: counts and allocation ranking |
| How does memory evolve? | `Memory` and `Scopes` dashboards, plus memory counter tracks |
| What did a reducer change? | Select `state · ...` or a causal message; open the bottom `State change` tab |

Use the search field **inside Perfetto**, not the browser address bar. Commands
can be selected with the mouse; do not depend on browser keyboard shortcuts.
For example, search for `state · todo_core::board::Board` in a detailed reference
recording to locate state updates.

`State change` offers **Diff**, **Update**, **Before**, **After** and **Go to cause**.
Selecting a causal message can show several reductions. The panel loads at most
100 matching updates initially and reports the total; select an individual state
event to inspect a particular update.

Normal mode has no captured values. Detailed mode uses serde when available and
honours its skipped fields. The current defaults are 64 KiB per snapshot or patch
and 256 retained history events in memory. Trace payloads are exported separately
from that bounded history. Capture limits and unavailable/failed serialization
are explicit. An empty patch means the **captured representation** did not
change, even though an update was applied and its revision advanced. It says
nothing about fields omitted from serialization. Enabling detail later cannot
recover past values.

## Interpret the measurements

- Event duration is elapsed execution, not sampled CPU time. Harness clock
  advances are virtual; the trace timestamps are actual recording times.
- A flamechart preserves time order. A flamegraph aggregates stacks; width is
  sample count for the `Sample Count` metric, not invocation count.
- `[Guinea]` frames are synthetic causal context. They start at the first
  resolved Guinea entry in the sampled stack. They are not evidence that an
  async sender remains physically on the worker's stack. Unresolved boundaries
  remain native; use `raw.pftrace` to inspect the original stack.
- Event counts distinguish action, send, handle, publish and delivery. Do not
  add these stages together and call the sum a count of business actions.
- Per-event allocation counters are direct, excluding nested event totals.
  Scope attribution describes allocation origin, not current ownership.
- Rust requested heap bytes, tracked scope bytes, tracing overhead, process
  working set and private commit are different measurements. Native heaps and
  allocator-reserved capacity are not equivalent to requested Rust bytes.
- Allocation turnover is not retained growth. Memory attributed to a closed
  scope can legitimately be owned elsewhere; it is a leak candidate, not proof.
- A relative source path or unresolved address must remain explicit. Do not
  guess a function name, declaration or source location.

For real Windows recordings, validate the sampling model before interpreting
sample counts as CPU time. Nonzero `threadCPUDelta` alone does not establish
uniform periodic sampling. The current `estimated_cpu_ms` and
`process_cpu_percent` queries use sample counts and the nominal interval; keep
those estimates unvalidated when the input's sample provenance is unknown.
A wait function at a stack leaf is not by itself proof that the row should be
dropped. Keep original ETL files, when available, for event-source analysis.

## Analyze with SQL

Use the official Trace Processor for native queries, or Perfetto's SQL editor.
The pinned Windows binary and checksum used for tests are in
[CI](../../.github/workflows/ci.yml). Native Trace Processor setup is separate
from the UI's bundled WASM engine; viewing a recording does not require it.

Start from the existing queries rather than recreating their attribution rules:

- [Hot CPU actions](../../crates/guinea-trace/sql/top_cpu_actions.sql).
- [Scope memory module](../../crates/guinea-trace/sql/guinea/memory.sql), loaded
  with `INCLUDE PERFETTO MODULE guinea.memory` and the SQL package registered.
- [Event totals](sql/events.sql.njk).
- [State tables](sql/state-module.sql.njk) and [state selection](sql/state-selection.sql.njk).

Fresh native SQL sessions do not inherit the plugin's materialized tables.
Load the appropriate module first. Use `renderQueries`, `renderStateQueries`
and `stateSelectionQuery` for parameterized templates; do not execute unresolved
Nunjucks markers or concatenate arbitrary application strings into SQL.

Identify event kinds through `debug.guinea_kind`, identities through
`debug.guinea_id` and causality through `debug.guinea_parent`. Display names are
presentation, not a schema. Use SQL track IDs for workspace membership, not
regular expressions over thread or lane labels.

## Verify changes and report findings

For exporter/stack changes, run the relevant tests in `guinea-trace`. For reducer
capture, run `state_recording` in the facade with `--features harness,perfetto`.
Keep regression snapshots for aliases and diffs; patches must replay `before`
into `after`. Preserve the normal-mode test that performs no state-value
serialization.

For the UI package:

```sh
npm --prefix tools/perfetto test
npm --prefix tools/perfetto run typecheck
npm --prefix tools/perfetto run format:check
```

The full viewer build additionally checks integration against pinned upstream
interfaces; the local package's typecheck alone does not cover every plugin file.
Official Trace Processor tests require `TRACE_PROCESSOR`; JS SQL integration also
requires `GUINEA_TEST_TRACE`. Follow CI's setup when running them. Ignored or
skipped tests are not completed verification.

Report the workload and recording mode, artifact directory, fresh viewer URL,
relevant measurements and evidence supporting the conclusion. Distinguish a
measured result from a hypothesis. State whether UI behaviour was actually
observed or only the build, tests and SQL were checked. Do not call a rendering
issue fixed solely because TypeScript compiled.
