# One todo application, five backends

`todo-core` owns the application. The other crates draw it with WinUI,
egui, iced, ratatui or Slint. Each is a separate workspace and has its own
lockfile, so testing one backend does not enable another backend's features.

From the repository root:

```sh
cargo run --locked --manifest-path examples/todo-egui/Cargo.toml
cargo run --locked --manifest-path examples/todo-iced/Cargo.toml
cargo run --locked --manifest-path examples/todo-tui/Cargo.toml
cargo run --locked --manifest-path examples/todo-slint/Cargo.toml
cargo run --locked --manifest-path examples/todo-winui/Cargo.toml
```

WinUI requires Windows and the Windows App Runtime expected by the bundled
reactor. The other frontends use their toolkit's usual platform requirements.
CI builds and tests all six crates on Windows; it does not open real windows.

## What to try

Add tasks, rename them, mark them complete, switch between the three filters
and inspect the activity log. Undo reverses the last edit; clearing completed
tasks is undoable too. Search follows changes to the board and replaces an
older pending query when you type again.

Import checklist streams three sample tasks with a delay between them.
Stop import cancels both the producer and the receiving stream. Starting a
new import replaces the previous one. Sync sends a typed RPC request to a
local demo service; it deliberately does not contact a remote server.
Cancel sync prevents its eventual reply from changing the displayed status.

The board is application-scoped, so navigation does not reset it. Task pages
are kept when inactive. WinUI and iced ask before discarding a changed title;
the other frontends discard that local draft on Back. WinUI also places the
detail page's Save action into a layout slot. Slint shares its draft and
query between filters because those controls belong to the same component.

In the terminal, `1`/`2`/`3` choose a filter, `l` opens the log, `n` starts
adding a task and `/` edits the query. Enter submits the input or opens the
selected task, arrows select a row, Space toggles completion and Delete
removes it. `u` undoes, `c` clears completed tasks, `i` imports, `x` stops
import, `s` syncs and `z` cancels sync. Escape goes back; `q` quits outside
text entry.

## Where the guinea concepts appear

| Concept | Application use |
| --- | --- |
| Application feature and exported reducers | `TodoApp`: board, save status, sync status, clock and journal |
| Page feature and scoped actors | `Search`, installed by task pages |
| Commands and reducers | `Editing` validates commands; `Board` stores snapshots |
| Event bus | `Changed` reaches storage and journal without actor addresses |
| Feed | Current board snapshot reaches search and the RPC client |
| Latest tasks and pushing | Search and sync replace earlier outstanding work |
| Named detached lane | Import producer runs without a reply but remains actor-owned |
| Streams and cancellation | Import consumer handles each arriving task |
| Request/reply and timeout | Local upload service answers the sync client |
| Timers | Application uptime and the save debounce |
| Plugin and cleanup | `DiskStorage` supplies a repository and closes the store |
| Routing and lifetime | Keyed task pages, details, history and kept page state |

## Persistence and plugin boundary

`DiskStorage` is a small plugin defined **in this example**, not
`guinea-plugin-store`. It uses the published amethystate 0.26 typed struct API.
The external store plugin's migration is not published yet; these examples
do not depend on a sibling checkout or an unavailable Git revision.

Each backend uses a separate JSON store under the platform's configuration
directory, selected by `StoreBuilder::located` for application `guinea-todo`
and its backend name. Edits are saved after 300 ms without another edit.
Application cleanup saves the latest snapshot even if that timer has not
fired, then closes the store. Undo history and activity entries are not
persisted. A failed save leaves the board in memory and appears in its status.

## Tests

```sh
cargo test --locked --manifest-path examples/todo-core/Cargo.toml
cargo test --locked --manifest-path examples/todo-egui/Cargo.toml
```

Use the same command with each other frontend's manifest. Core tests run the
real actors under guinea's seeded harness: CRUD, undo, debounce, latest search,
feed wake-up, import cancellation, RPC and application lifetime. File-backed
tests reopen the JSON store and check saving at shutdown before the debounce.
WinUI and egui tests mount their actual page trees and dispatch UI actions;
Slint tests invoke the generated component's callbacks on its testing backend.
The iced and terminal crates also test their local guard and row formatting.
