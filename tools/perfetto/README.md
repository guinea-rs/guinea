# Guinea profiling in Perfetto

Agents: start with [the profiling workflow](AGENT_GUIDE.md) for recording,
analysis, UI navigation and verification.

Install the standalone profiler once:

```sh
cargo install --locked guinea-profile
```

For an existing application built with symbols:

```sh
guinea-profile record -- path/to/app.exe app-arguments
guinea-profile process path/to/unprocessed-recording
guinea-profile view path/to/recording
```

`process` converts existing inputs; `record` already does this after recording.
The binary never invokes Cargo to build the application or perform conversion.
See [the CLI guide](../guinea-profile/README.md) for symbol builds, cache and
portable UI assets.

For the repository's reference workload:

```sh
cargo xtask profile
```

The xtask wrapper builds the todo application reference session with symbols and
launches `guinea-profile`. The profiler records it with samply, resolves its
symbols, combines the samples with Guinea's causal trace, and opens the stock
Perfetto UI with the Guinea plugin. The first run prepares a pinned Perfetto
build in a separate tool cache; later runs reuse it.

The reference uses `examples/todo-core`, the same application layer as the five
GUI frontends. It adds 1,500 tasks, edits and completes them, replaces search
requests, updates search from the application's feed, clears completed tasks,
undoes the change, saves through the real amethystate JSON plugin, closes the
search scope, tears down the app and verifies the persisted board after reopening.
CPU stacks, allocation counters and process memory belong to this one recording.

`combined.pftrace` shows Guinea's semantic frames at the first resolved entry
into a Guinea crate. Native callers stay above that boundary, application
functions stay below it, and Guinea wrappers are collapsed. Public method names
are not used to identify the boundary. A stack with no resolved Guinea boundary
is kept unchanged. `raw.pftrace` preserves the original sampled stacks without
aliases or synthetic ancestors; the original samply recording and symbols also
remain in the recording directory.

Guinea event lanes carry the emitting thread's Rust `ThreadId` and name. On
Windows and Linux they also carry the native thread ID, shared with sampled CPU stacks.
On Windows, the WinAPI thread description is used when no Rust name is available. Rust's
`main` thread is labeled `Main thread`. The original names and identity mapping
are retained in `rs.guinea.thread.*` trace metadata. Other platforms currently
keep native CPU stacks without Guinea attribution when native thread identity
is unavailable.

The harness drives the session and advances debounce deadlines deterministically.
The trace contains actual elapsed execution time, not the harness's virtual wait
durations. It covers application logic and storage; GUI layout and rendering are
outside this reference. No synthetic action/handler spans or busy loops are used.

For reducer values and structural state diffs, explicitly enable detailed capture:

```sh
cargo xtask profile -- 64 --state-details
```

Select a `state · …` event or its causal message on the timeline, then open
`State change`. The panel shows the update, RFC 6902 diff and before/after
snapshots, and links back to the cause. Unchanged state, unavailable serialization
and capture size limits are explicit. Normal mode records metadata only. Payloads
are stored in the trace's metadata, with no payload tracks; snapshot and patch
sizes and in-process history are bounded.

Profile an application instead:

```sh
cargo xtask profile --manifest-path ../app/Cargo.toml -p app --bin app
```

The application must attach `guinea_trace::perfetto::from_env(&sink)` to its
existing sink and finish the recording before exiting. The command sets the
recording path and symbol options only for the child process. Its Cargo
`profiling` profile enables symbols without changing ordinary build settings.

Reopen an existing recording:

```sh
guinea-profile view profiles/<recording>
```

No dashboard import or manual query execution is required. Keep the command
running while using the viewer; Ctrl+C stops its local server. `--no-open`
prints the URL without opening a browser. `--no-ui` records and converts only.

## Requirements

On Windows: a working WSL Linux distribution for Perfetto's official UI build,
and the Windows Performance Toolkit (`xperf`) for samply. CPU recording may
require an elevated terminal. The command installs missing samply into a local
tool directory, not globally. It does not install system components or alter
security settings. The first setup needs network access.

## What the plugin owns

- English metric titles and dashboard presets: `src/catalog.ts`.
- SQL queries and reusable Nunjucks includes: `sql/`.
- Dataset initialization and schema discovery: `src/session.ts`.
- Thin integration with Perfetto's existing Dashboard and flamechart tracks:
  `src/index.ts`.
- Trace capabilities: SQL probes the loaded Trace Processor database, so
  CPU-only recordings do not get memory dashboards and cached traces work too.

The memory module and hot-action attribution query come directly from the
tested SQL package in `crates/guinea-trace/sql`. Nunjucks renders queries at
build time. Its templates never interpolate application values. The UI
materializes datasets and discovers columns before rendering charts; SQL
failures fail initialization instead of silently displaying empty graphs.

`build.sh` pins the upstream revision and installs only this plugin into an
isolated checkout. It leaves Perfetto's core UI unchanged. The local viewer
serves the recording and UI on one origin and binds only to loopback.

The current target is Perfetto v59.1 (`79997ada96b2587556cfc3851434407fbed289b1`).
Dashboard is an upstream internal component, so updating this pin requires
building and checking the plugin against that version.

## Checks

```sh
npm ci --prefix tools/perfetto
npm --prefix tools/perfetto test
npm --prefix tools/perfetto run typecheck
cargo test -p xtask -p guinea-profile
```

The upstream UI build also type-checks the plugin against the pinned Perfetto
interfaces. To exercise rendered SQL against a real trace, run the JS tests
with `TRACE_PROCESSOR` pointing to the official executable and
`GUINEA_TEST_TRACE` pointing to `combined.pftrace`.
