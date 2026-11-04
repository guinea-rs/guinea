# guinea-profile

Standalone CPU and Guinea profiler. Build your application separately; this CLI
launches an executable, records with samply, resolves symbols, converts recordings
in-process and serves stock Perfetto with the Guinea plugin.

```sh
cargo install --locked guinea-profile
guinea-profile record -- path/to/app.exe app-arguments
guinea-profile record --no-ui --rate 250 --output profiles/session -- path/to/app.exe
guinea-profile process path/to/unprocessed-session
guinea-profile view path/to/session
```

`merge` remains an alias for `process`. Processing preserves source recordings
and refuses to overwrite processed output. `record` processes automatically;
`view` only opens an existing `combined.pftrace`. Recording directories default
to `profiles/<timestamp>-<pid>` under the caller's working directory. Application
arguments are forwarded without a shell and the app inherits the working directory.
Use `record --dry-run -- <executable> ...` to inspect the launch command.

## Recovery without another recording

```sh
guinea-profile process saved-session --output recovered-session
guinea-profile process saved-session --output recovered-session --executable path/to/app.exe --symbol-dir path/to/pdbs
```

`--output` must be new. CPU/Guinea inputs and optional session/sidecar files are
copied unchanged. The saved session supplies the executable path when available.
Symbolication runs automatically for both `record` and `process` using wholesym,
with recorded module debug identities, local binaries/PDBs, symbol directories
and the standard Windows symbol path/cache. It never fabricates symbol names.
The application is processed first; `--symbol-timeout` sets a total lookup budget
(default 120 seconds). Missing/incomplete application symbols fail conversion
with a diagnostic; `--allow-unresolved` explicitly permits a partial result.
System modules can have unavailable/private symbols and are reported as partial.
`symbolication.json` records per-module resolved/unresolved counts and errors;
`cpu.resolved.syms.json` contains generated symbols. Original sidecars are retained.
Windows recordings keep ETL inputs (`--keep-etl`) for later source-event analysis.

## Symbols

Supply a binary built with symbols and its PDB/debug files. For a Cargo project:

```sh
cargo --config 'profile.profiling.inherits="release"' --config 'profile.profiling.debug=2' --config 'profile.profiling.strip=false' build --profile profiling --bin app
```

On MSVC, linker flags must also allow PDB generation. A global `/DEBUG:NONE`
can override Cargo's debug setting; configure the profiling build specifically.
The repository's `cargo xtask profile` wrapper handles that, including an absolute
PDB lookup path, for the reference application. It then delegates to this binary.
The profiler does not change the application's build settings.

The application must attach `guinea_trace::perfetto::from_env(&sink)` and finish
the recording before exiting. The profiler sets `GUINEA_TRACE_FILE` for the child.
Without a Guinea trace, the CPU recording and Perfetto-compatible CPU JSON remain
available, but no correlated combined trace is produced.

## Tools and UI assets

An existing samply 0.13.1 on PATH is used. Otherwise the CLI installs that version
with Cargo into its own cache. Windows CPU recording requires Windows Performance
Toolkit and may require elevation. The cache defaults to LOCALAPPDATA on Windows,
XDG_CACHE_HOME or HOME/.cache on Unix, under `guinea-profile`; override it with
`--cache-dir`. System symbol downloads are cached there too.

On Windows, packaged sources are streamed into WSL's native cache; viewing does
not depend on WSL being able to mount the Windows cache drive. The build publishes
an immutable UI distribution so another build cannot replace an open viewer's
files. Guinea is included in the stock embedder's default plugin configuration.

The binary embeds the pinned UI integration sources, SQL templates, dependency
lockfile and Guinea SQL. No Guinea source checkout is needed. On first use it
extracts them to a versioned content directory: under the configured cache on
Unix, or WSL's native cache on Windows. Subsequent runs reuse these sources,
restoring missing or changed files. The
first UI build downloads the pinned upstream Perfetto checkout and dependencies,
so network access and the build prerequisites are still required. On Windows the
UI build runs under WSL. Upstream checkout and built UI caching are managed by
the bundled build script under the build environment's HOME/.cache/guinea-profile.

For integration development, `--integration <path/to/tools/perfetto>` overrides
the embedded sources. Keep the accompanying repository-relative Guinea SQL files
available for this override. To install from a local checkout instead, use
`cargo install --locked --path tools/guinea-profile`.

A relocated binary can use an already built UI without Cargo, WSL or the checkout:

```sh
guinea-profile --ui-dir path/to/perfetto-dist view path/to/session
```

Pass the complete built distribution, including WASM and the Guinea plugin.
The viewer binds to loopback and runs until Ctrl+C. `--no-open` prints its URL
without opening a browser; `record --no-ui` skips UI preparation entirely.

See [the agent guide](https://github.com/guinea-rs/guinea/blob/master/tools/perfetto/AGENT_GUIDE.md) for analysis and metric semantics.


For maintainers: `tools/perfetto` and `crates/guinea-trace/sql` are the canonical
sources. Run `pwsh -File tools/guinea-profile/update-integration.ps1` after changing
them, then `cargo test -p guinea-profile --bin guinea-profile`. The sync regression
checks packaged copies against those sources; `build.rs` embeds only the packaged
assets and also works when building the published crate outside the repository.

UI source builds require Bash, Git, Python 3, `flock`, `sha256sum`, and `sort`
with `-z` support (plus the upstream Perfetto build prerequisites). Linux/WSL
must have these installed. macOS does not supply `flock` or `sha256sum` by
default: install GNU coreutils and a `flock` implementation and put their commands
on PATH before building. The build script reports missing prerequisites before
downloading Perfetto. Alternatively, `--ui-dir` serves a complete prebuilt UI
without invoking the build script or requiring those tools.

## Symbolication dependency

The CLI uses registry wholesym 0.8.1. Its transitive ruzstd dependency is covered
by RUSTSEC-2026-0344 (decompression resource exhaustion). This advisory is
explicitly accepted for this release until wholesym publishes its upstream
upgrade. Process trusted recordings, binaries and symbols.
