# TODO

## Devtools (guinea-plugins)

### The MCP server answers in JSON meant for a window

`tools/devtools/mcp` offers every route of the API as a tool, generated from
the OpenAPI document, and hands back the response body as it came. That body
is shaped for the window: one trace record is an array of words with tones and
links, ten lines of JSON for what reads as
`14:47  push into Metrics  <- MetricsActor handles Tick`. The only consumers
are agents, and they pay for every line of it.

Left:

- A text rendering per operation, chosen by the operation's id, with the raw
  body behind a `json: true` argument for the routes where structure is the
  point (`get_element`, `get_native_tree`, `get_graph`). `words::text` in the
  model is the primitive; the printers the deleted CLI had are worth lifting
  rather than rewriting - `git show HEAD:tools/devtools/cli/src/main.rs`.
- Tool descriptions written for an agent, saying what the answer is good for,
  instead of the `summary` that `--help` prints at a person. Which means the
  description stops being derived from the summary.
- `.mcp.json` in the root of `guinea-plugins`, so a session started there sees
  the tools. A session picks its tools up at startup, so registering one only
  reaches the next session, not the one that registered it.

### A profiler, as the backend's

The puffin profiler is cut from core. It zoned only `Rendering`, and only
eframe and winui ended a frame, so it was a puffin stub for two backends
living in the core every backend builds on.

What a frame is and where it goes is the backend's to know: a toolkit's own
profiler (puffin for egui, XAML's for WinUI, Slint's) or a frame boundary the
backend reports. To decide: whether core offers anything here beyond the trace,
which already times handlers, pulls, background work and rendering.

Devtools still has `Capability::Profiler`, `Command::Profiler`,
`Report::Profiler` and a `puffin_http` server behind its `profiling` feature,
which named `guinea-core/profiling`.

## Deep links

Found by running the generated `link()` / `parse()` over edge cases.

### Bug: an empty `String` capture does not round-trip

`Route::Host { host: "" }.link()` is `"//processes"`, and `parse` answers `None`
for it: the generated walk splits on `/` and drops empty segments, so the
capture disappears. The application hands out an address it does not answer.

Fix: keep empty segments, strip only one leading and one trailing `/`. This
stops `//ubuntu//processes` from matching, which it does today.

### Gap: nothing opens a link

- `parse` takes a path only; `guinea-processes://ubuntu/processes` and
  `guinea-processes:///ubuntu/processes` both give `None`.
- How scheme and path compose is not defined. In `scheme://ubuntu/processes`
  `ubuntu` is the authority, not a path segment, so the form has to be
  `scheme:///…` or `scheme:/…`.
- No adapter or example reads the activation address (argv, Windows protocol
  activation). There is no way from a URL to a screen.

### Smaller

- Numbers and `bool` do not unescape: `/job/%34%32` is `None`, while strings
  are unescaped. Unescape first, then parse.
- Non-canonical numbers are accepted: `/job/+42`, `/job/042` and `/job/42` are
  one route, `/offset/-0` too. Require the segment to equal
  `value.to_string()`.
- Literals are not unescaped: `/ubuntu/%70rocesses` is `None`, though RFC 3986
  makes `%70` and `p` equivalent.
- `.` and `..` captures round-trip here, but any URL normaliser removes them
  first, and WHATWG treats `%2E%2E` as `..` as well. Probably refuse in
  `link()`.
- A path with `?query` or `#fragment` is `None`. Fine if intended; some shells
  append one.

## Table (guinea-widgets, guinea-plugins)

Found after the table moved from `ListView` to `ItemsRepeater` +
`VirtualSource`, measured on uniproc.

### Bug: a click handled inside a cell also selects the row

The row selects itself from `Border::on_pointer_released`. A cell that handles
its own click - uniproc's chevron that folds a group, a `Border` with
`on_pointer_released` - fires, and then the row's handler fires too, selecting
the row. `ListView` did not do this.

Nothing in the table can stop it: reactor pointer callbacks are not routed, so
a handler cannot mark the event `Handled`. Keys can (`RoutedCallback`),
pointers cannot.

Options:

- Ask reactor for routed pointer callbacks, as keys have. The real fix.
- A column that does not select (`ColumnSpec::inert()` or the like). Coarse:
  the chevron shares its column with the name, which should still select.
- A "handled" flag the cell sets and the row checks. Works only if reactor
  runs the inner element's handler before the outer one; not verified.

### Regression: rows are not list items to UI Automation

With `ListView` the body was list / list item with a selection pattern. Now it
is one group with a flat run of text and images: no rows, no selection, for
screen readers and automated tests alike.

Reactor offers `automation_name`, `automation_id` and `automation_heading_level`
only - no control type, no selection pattern. Nothing to fix on the table's
side until reactor has them.

### Smaller

- The selection highlight ends at the last column, not the window's edge.
  Each realized row sits in a pooled `ContentControl` whose
  `HorizontalContentAlignment` stays `Left` from the built-in style. Setting it
  to `Stretch` live through XAML diagnostics made the highlight full width, so
  the fix is in reactor's shell.
- Column resize was not checked after the rewrite; synthetic input did not
  move the handle. Check by hand.

## WinUI adapter

### `PageCx` and `LayoutCx` need doc tests

`crates/adapters/guinea-winui/src/winui.rs`. Their docs are prose only: no
`<!-- shown: .. -->` example taken from `tests/docs.rs`, so nothing the reader
sees is compiled or run, and nothing checks what the prose says.

`PageCx`:

```rust
pub fn on<T>(&self, message: impl Fn(T) -> P::Message + 'static) -> Callback<T>
where
    T: 'static,
{
    self.cx.callback(move |payload| Signal::Node(message(payload)))
}

pub fn use_reducer<R, I>(&mut self) -> (Rc<R>, guinea_core::feature::Dispatch)
where
    R: Reducer + PartialEq,
    P: Reaches<R, I>,
{
    use_reducer::<R, _>(&self.props, self.cx)
}

pub fn navigate<R>(&mut self) -> NavigateHandle<WinUi, R>
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    self.cx.use_navigate::<R>()
}
```

- `on`: a widget's payload comes to `update` as the page's own message.
- `use_reducer`: the page reads `R` from its own scope or the nearest one
  above that exports it, redraws when it changes, and its `Dispatch` reaches
  the feature that owns `R`.
- `navigate`: the handle goes to the route asked for.

`LayoutCx`, the whole `impl<L: Layout> LayoutCx<'_, L>`: `on`, `outlet`,
`child_is`, `open_window`, `navigate`, and `use_reducer` from the
`L: Layout + Segment` impl beside it. Most of them say only "See
[`PageCx::..`]", so a layout's own example is what they lack:

- `outlet`: the segment below is drawn where the layout puts it.
- `child_is`: a tab strip lights the tab of the page below it.
- `open_window`: a window opens from the layout, not from `view`.

## Windows: restored geometry

### WinUI has no window contract

Undecided what to do with it; written down 2026-10-19.

`guinea_app::app::windows` has one implementer, the Slint shell, and one
consumer, the window-state plugin, used only by `processes-slint`. WinUI
provides no `WindowService` and never reads `SavedGeometry`, so window-state
does nothing there. `PluginBuilder::windows()` has no callers.

What windows-reactor-pre (vendored) offers now:

| `Windows` needs | the reactor has | |
|---|---|---|
| `geometry()`, `WindowChanged` | `cx.on_window_placement` - restored outer bounds in physical pixels and `maximized`, coalesced, nothing while minimised; `cx.on_window_size` - client size in DIPs | yes |
| restore before showing | `WindowVisuals::initial_placement(WindowPlacement)`; Windows moves fully off-screen bounds onto a monitor itself | yes |
| `apply()` after opening | `client_size` stays live in the declaration; position and maximized are read only at creation | size only |
| `close()` | `ComponentContext::close_window()`, from the window's component on the UI thread | needs a bridge into `Chrome` |
| `set_visible`, `start_drag`, fullscreen, minimised | nothing | no |
| escape hatch | `cx.run_window(\|handle\| ..)` hands out the raw HWND | a workaround only |

Partial implementation, if it is done: a `WinuiWindows` keeping each root's
last placement and size, fed by both observers in `Chrome`, which also
publish `WindowChanged` with the label; `geometry()` answers from it; the
shell reads `SavedGeometry` from the application's services and sets
`initial_placement`; `apply()` takes the size only, the rest answers
`Unsupported`; `run` provides `WindowService` as the Slint shell does. No
Win32 through the raw HWND - the missing commands are a draft issue for
windows-rs instead.

Open questions before that:

- `Geometry` and `WindowPlacement` disagree on units. `Geometry` is the
  winit / Slint shape: a logical client size and a physical outer position.
  The reactor keeps whole outer bounds in physical pixels, the way Win32's
  `GetWindowPlacement` does - the one that survives a restart without loss.
  Carrying WinUI's placement through today's `Geometry` means converting
  DPI and frame insets both ways, which drifts. Moving `Geometry` to outer
  physical bounds changes `fit` and what window-state stores - breaking for
  the plugin.
- The Slint shell is handed restoration twice: window-state provides
  `SavedGeometry` as a service, and `Window::remembered_by(saved)` takes it
  again by hand. The shell could read the service itself and
  `remembered_by` go.
- `close()` now through a bridge into `Chrome`, or `Unsupported` too.
- Or none of it: the contract is speculative while no application uses it,
  `fit` belongs inside the Slint shell, and the module could leave
  guinea-app until a second real implementer shapes it.

## Versions: a plugin should not break on a guinea it does not use

Where to cut is not clear yet; nothing here is decided. Replaces the lockstep
version agreed on 2026-10-15 once it is.

### Each crate its own version

Drop `version` from `[workspace.package]`; every crate carries its own, all
starting from the same number. `cargo semver-checks` per crate in CI says
which ones need a minor bump; an `xtask release` lists the crates changed since
the last release, proposes their bumps and raises the requirement in their
dependents (not release-plz: it wants conventional commits). With it: `links`
on guinea-core, so two cores in one tree do not build, and macro paths that
prefer `[dependencies]` over dev-dependencies.

On its own this saves no plugin. When the core breaks, every crate that
re-exports its types breaks with it, guinea-app included, and a plugin on
guinea-app breaks anyway. Separate versions only say which crate broke; they
help a plugin as far as it depends on crates that do not drag the core along.

### A layer the plugins stand on

What the plugins take from guinea, counted on 2026-10-16 (on 0.29):

- l10n 9 paths, store 7, window-state 5, single-instance 5: `Plugin` and its
  builder, `Scope`, `Reducer`, `Dispatch`, the window types (`Geometry`,
  `SavedGeometry`, `WindowChanged`), `roots::label`, `AppMeta`, `Stop`, the
  observability panels, `trace::Point`, and the harness in their tests.
- widgets: only `guinea_mark::Mark` - a small crate that rarely changes, which
  is what the layer should look like.
- devtools: 30, all of them the core's insides (actor registry, shape, remote,
  panels, trace). A tool tied to the core, not a plugin; its stable layer is
  `devtools-protocol`, not guinea's API.

Two halves to the plugins' contract:

- Data: window geometry, a root's label, `AppMeta`, panels, trace points. These
  can go into small crates with their own versions, as `guinea-mark` did, and
  stop moving with the core.
- Behaviour: `Plugin`, the builder, installing features, reducers, `Dispatch`.
  This is the feature and actor model itself, and while the review rewrites
  it no layer makes a plugin stable. Moving the vocabulary down, Essenty-style
  (the traits in a bottom crate, the core implementing them), takes `Feature`
  along and half the core's public surface after it. Design it after the
  review, once the model holds still.

## Examples

### A todo app with everything in it

`examples/` has the processes app, which shows guinea on a real workload but
not every part of it at once. Write the canonical todo app, using all of it:
routes with a deep link and a restored route, a guard that asks before leaving
unsaved work, features that install and tear down with their pages, actors and
background work, the store, localisation, devtools, and the harness testing
all of it. What a newcomer reads first.

### When there is nothing else to do: rewrite open source on guinea

Take random open-source Rust GUI applications, rewrite them on guinea, and
write up a comparison: how much code, what got simpler, what got harder, what
guinea was missing. Each one is a test of the design against an application
nobody wrote for guinea.

## Actors

### Suspected: actors are not released (not reproduced)

Seen in uniproc as actors subscribing to the same events again and again.
Checked, and freed every time - the actor's state dropped and its global
subscription gone:

- a segment's actor under the harness, idle and after handling messages, a
  background answer and a timer (`harness.rs`, `an_actor_*_let_go_of_*`);
- a page's actor and a layout's feature through the real router and WinUI
  mount, three visits in a row (`released_winui.rs`);
- router navigation away from a page (`routing.rs`,
  `navigating_away_from_page_disposes_actor_subscribed_to_the_bus`).

What uniproc shows is most likely the design: its domain features are
installed by the area layouts (`processes_area` installs
`ProcessesFeature`), so every visit to an area installs a new actor that
subscribes anew, and leaving tears it down. An area that should keep its
actor across visits is a `keep` segment.

An actor lives exactly as long as the scope it was made in: the scope holds
it and disposes it when it goes, and nothing else can. One that outlives
what it was made for was made in a scope that lives too long. If a leak shows
up again, measure it first: `ALIVE` counters on the actor's state.

## Router: one way for a page to outlive leaving

Decided in conversation 2026-10-13, not started.

### `CACHE_STATE_IN_MEMORY` goes; `keep` is the only way

Two mechanisms do one thing today:

| | `keep` in `routes!` | `const CACHE_STATE_IN_MEMORY` on `Page` |
|---|---|---|
| Declared | at the segment's place in the tree | on the `Page` trait, in all six adapters |
| What outlives leaving | the whole scope, asleep: reducers, sections, actors | reducer cells only: the router snapshots them on leave and restores them into a new scope (`snapshot_states` / `restore_states`) |
| Actors | sleep and wake as the same actors | made anew, with whatever they mirrored back at defaults |
| Params | woken only for the same capture | restored only for the same capture |
| Limit | only right under what stays (`kept_until`) | anywhere |

The cache is the weaker copy: it moves state out of the scope into the
router and leaves the actors behind, so an actor's fields and its reducer
disagree after a return - uniproc's Services sorts from `Name` and
`Command` goes without a target. The only thing the cache does that `keep`
cannot is outlive its parent, and no caller needs that: all three users
(uniproc `Services` and `Wsl`, processes-iced `Metrics`) sit right under a
layout that stays.

- Delete the const from every `Page`, `SegmentEntry::cache_state`, the
  router's `state_cache` and `Cached`, and `snapshot_states` /
  `restore_states` in `scope/state.rs`.
- iced keeps a page's node by `keep` instead of by the cache flag
  (`nodes.rs`).
- processes-iced `Metrics` moves to `keep`; its module doc says what the
  cache covered and is rewritten.
- Test first: back on a `keep` page, the same actor answers with the state
  it had.
- Breaking: a minor bump.
- Open: `keep` stays in `routes!` (whether a segment can be kept depends on
  where it sits), or moves to `#[page(keep)]` - one place either way, never
  both.

### Not doing

- `cx.read::<R>()` for actors. Reading turns the flow around - actor, push,
  reducer, UI - and makes the reducer shared memory with its own questions
  (does a read after a push in the same turn see it). The mirrors that asked
  for it come from the cache splitting lifetimes and from view state living
  in features; both go without it. Back on the table only with a caller that
  cannot be written otherwise.
- A topology builder for `driven_by` (1:N, N:1, N:M). Every writer in uniproc
  is 1:1, and no live N:1 case is in sight. `docs/topology.md` is the
  rejected draft.

### For uniproc (its session's)

- `Services` and `Wsl` declared `keep`, each with a wake hook that catches up,
  as `ProcessesArea` does.
- Actions carry their target - `Command(name, kind)`, `Terminate(pid)` - so
  the selection is a field of the page, not of the feature.
- Sort is the actor's input, taken from `Sort`; not mirrored into the reducer.
- `MachineSummary` is computed by one feature and exported to the other;
  today processes and metrics both compute it from the same report.

## Harness

### `settle` is something to remember

Half of the harness API leaves the page unsettled, and a test has to remember
to settle it - uniproc's `processes_settings.rs`:

```rust
page.send(ProcessesSettingsMsg::Expand(Group::Columns, true));
page.settle();
page.within(ProcessColumn::Pid).click(ProcessesSettingsMark::Shown).settle();
page.settle();
```

`send` runs one turn and settles nothing. `click` hands back an `Act`, whose
`settle` waits for what the action set off but not for the page to draw it
again - so a second, bare `page.settle()` follows. Forgetting either still
compiles, and the assertion after it reads a page that is one step behind.

Worth sitting down with. Directions, not decided:

- `#[must_use]` on `Act`: cheap, catches the dropped act, not the missing
  redraw.
- Settle by default: `send` and `click` return with everything settled, and
  a test that wants the page in between asks for it (`click_unsettled`, or an
  `Act` that has not run yet).
- `Act::settle` settles the page too, since an act from a mounted page belongs
  to it.
- Reads settle first: `find`, `property`, `tree` run what is pending before
  they look.

### Tests take turns over global state

uniproc's page tests run as `#[guinea::test(iterations = 4, exclusive =
"store")]`: the store is one per process, so tests that touch it cannot run
side by side, and each seed has to put it back. `exclusive` works, but it is a
lock around a global, not an answer to it. The same goes for everything else
the process holds once - `L10n::<S>::load`, the global event bus the harness
resets per seed.

Sit down and look at the options for getting rid of global state: a store the
application `provide`s and features `require`, so a harness hands each seed
its own; globals keyed by the application they belong to; or what else there
is.

## Localisation (guinea-plugins)

### `L10nPlugin::new` takes any string

`plugins/l10n/src/lib.rs`:

```rust
impl<S: Localization> L10nPlugin<S> {
    /// `default_tag` is a BCP-47 language tag - `"en"`, `"ru"`, `"pt-BR"`.
    pub fn new(default_tag: impl Into<String>) -> Self {
        Self {
            default_tag: default_tag.into(),
            persist: cfg!(feature = "persist"),
            resolver: PhantomData,
        }
    }
```

That it is a language tag lives in the doc comment only. `new("english")`
compiles, and the mistake surfaces at startup at best: if `S::for_tag`
refuses it, `build` fails and so does the application. Work on correctness: the invariant belongs in the
type - a language tag that can only be made valid, checked at compile time
for a literal.

### The persisted key is two bare strings

```rust
#[cfg(feature = "persist")]
const KEY: [&str; 2] = ["app", "language"];
```

`StorePath` has a const builder; the key should be a `StorePath` made by it,
not an array the store reads a path out of.

## Core: the scope before the router

Decided in conversation, not started. Three wants need the same change: a
router that stays out of the way, devtools that see all of the core, and
backends that bring their own navigation.

### Scopes are created by whoever hosts them

Today only the router creates scopes, and every adapter must start from a
route: `guinea_eframe::run` takes `initial: impl FnOnce() -> R` with
`R: RouteChain<_>`, and each of the five adapters carries a `nav.rs`. Make the
scope the core's own primitive and the router one host among several - the
application, a window, a Tauri webview, a Dioxus component. An application with
no routes runs with one root scope.

The devtools snapshot has the same shape problem: it is roots, then chains, then
segments, and everything else hangs off a segment index. Make it a scope tree,
with the router as a note on the scopes it holds.

What the snapshot does not show yet:

- an actor's mailbox depth and the message it is handling now;
- live background tasks - today devtools rebuild them from the trace
  (`tasks.rs`), so after a reconnect, or with the trace off, they are gone;
- plugins and the `provide` / `require` graph;
- what a scope will let go of (its `Taken`: actors, subscriptions, timers,
  installed features, cleanups);
- held navigation (`Held` between `drawing` and `settle`) and the
  `notify::turn` queue.

Worth copying from Bevy BRP: `+watch` subscriptions to changes, and methods
that plugins register themselves, so a plugin shows its own insides through
the same channel. Free while nobody is watching, as the trace already is
(`sink::wanted()`).

### Plugin API: Tauri's shape, Bevy's composition

After the scope change, since window hooks need scopes a host creates.

From Tauri, take the v3 names, not v2 (`run_invoke_handler`, not `extend_api`):

| Tauri | guinea |
|---|---|
| `setup(app, api)` | today's `build` |
| `on_event(RunEvent)`: `Ready`, `ExitRequested { prevent_exit }`, `Exit` | an application event enum, which does not exist yet |
| `on_window_ready` / window destroyed | a root scope created / torn down |
| `on_page_load` | a segment mounted, when there is a router |
| `on_drop` | the plugin torn down |

- Extension traits on the context (`cx.store()` instead of
  `require::<Store>()`), the way `impl<R, T: Manager<R>> XExt<R> for T` works.
- A plugin's actors reachable from outside under its name
  (`plugin:store|...`), for the Tauri bridge and for devtools and MCP.
- A typed plugin config.
- One monorepo, one layout, a `plugin new` scaffold, and plugin majors that
  move with guinea's.

Do not take: a runtime generic on every plugin (`Plugin<R>` - Tauri itself
moves runtime-specific API into extension traits in v3; here backend-specific
parts go through their own extension points, like devtools'
`winui.components` panels); a panicking `state::<T>()` (`require` returning
`Result` is better); config as `serde_json::Value`; "the last
`invoke_handler` wins"; permissions, until there is a trust boundary.

Tauri has no ordering - registration order is the order, and its docs say
`single-instance` must come first. Take that part from Bevy instead: groups with
`add_before` / `add_after` / `disable` / `set`, a `ready` phase for a plugin
still coming up, `finish` once all are built, and `is_unique`.

The biggest obstacle to third-party plugins is not in the API: plugins pin a
guinea rev, and an application has to patch guinea to that same rev or build
two `PluginBuilder`s. A small, semver'd plugin-API crate on crates.io removes
it.

### Tauri: guinea as one Tauri plugin, actors instead of commands

- A Tauri plugin (`tauri::plugin::Builder`) whose `setup` builds the
  `GuineaApp`, sets the dispatcher to `AppHandle::run_on_main_thread` and
  `provide`s the `AppHandle`; `on_webview_ready` creates a scope for the
  webview label, and closing the window tears it down; `on_event(Exit)` stops
  the actors.
- `invoke_handler` is a plain `Fn(Invoke<R>) -> bool` (from memory - check):
  route `plugin:guinea|ProcessActor.Kill` to an actor by a table generated
  from what `actor!` already declares, and answer through `invoke.resolver`
  once the actor replies.
- Cancellation, which Tauri lacks (issue #8351): an id per `ask`, a JS
  `AbortSignal` sends a cancel that fires that request's `Cancel`.
- The trace crosses IPC: an `invoke` is a root; send, handle and publish
  follow; `emit_to` goes back to the frontend.
- ACL: plugin command permissions are generated in the plugin's `build.rs`
  from its command list (`tauri_plugin::Builder::new(COMMANDS)`); codegen
  supplies that list.
- No pages and no router here: `View = ()`, and the frontend routes. What goes
  to the frontend must serialise.
- The line for users: OS access from the webview is a Tauri plugin; logic with
  state, lifetimes and causes is a guinea feature or plugin. Tauri plugins'
  Rust APIs (`app.dialog()`, `ShellExt`, ...) are services guinea features
  `require`.
- Check before relying on it: CrabNebula DevTools is known to clash with
  other loggers, and guinea installs its own subscriber (`init_subscriber`).
  Tauri 3 is in alpha and moves the plugin API; start on 2.x and keep this
  layer thin.

### Dioxus: without guinea's router

Dioxus keeps navigation (`dioxus-router`); guinea brings features, actors and
reducers.

- `use_scope::<Features>()`: `use_hook` installs, `use_drop` tears down, so a
  route mounted through `Outlet` gets its features and loses them, tasks
  included, when it unmounts - the same rule Dioxus applies to `spawn`.
- `use_reducer::<R>()` returns a `ReadSignal` fed by a subscription to the
  reducer, so Dioxus keeps its own reactivity.
- `use_addr::<A>()`.
- The dispatcher is a channel drained by a `spawn_forever` coroutine. Each
  window is its own VirtualDom, so application-level work must target the main
  one.

### From the research into other frameworks

- The deterministic harness is in (`crates/guinea/tests/harness.rs`,
  `harness_winui.rs`): `guinea_core::executor` is the seam everything guinea
  spawns goes through; `app::Harness` runs background tasks and answers bound
  for the UI thread on the test's thread in a seed's order, and
  `#[guinea::test(iterations = N)]` names the failing seed for `SEED=` to
  replay. Its clock is a paused tokio runtime, so `tokio::time` in the code
  under test and guinea's own timers move by `advance`. `act` / `settle` wait
  for one action's consequences; `chain().shape()` is a snapshot of what it
  set off; `child()` / `leave()` are segments; `guinea_winui::harness::Mounted`
  mounts a page on reactor's recording runtime and reads back what it drew.
  Elements are reached by `Mark` - an application's enum, derived, put on as
  `AutomationId` with `.mark(..)`; `within(mark)` and `item(index)` (a list
  item, realized as scrolling to it would) narrow where the next find looks,
  so one mark in every row still names one thing. Text is `find_text` /
  `click_text`, for reading back what was drawn.
- Driving a running application from devtools, and so from MCP, is in:
  `#[derive(guinea::Remote)]` with `#[remote(action)]` / `#[remote(event)]`
  registers a type in `guinea_core::remote`; `guinea::observability::act` sends it
  to the scope on the open page that answers it, page first and layouts
  after. The hub's `send_action` / `publish_event` wait for the answer and
  follow the cause through the trace until nothing under it runs and nothing
  new comes of it for a moment; `find_element` / `click_element` /
  `type_into_element` go through the XAML tap by mark, with the real pointer
  or through UI Automation. Left:
  - Reducer state reaches devtools as `Debug` text only; reading it back
    structured needs the same opt-in as actions (`Serialize`).
  - Only scopes under a router answer: an action for a feature installed on
    the application itself has nowhere to go.
  - Keystrokes are text only - no Enter, Tab or shortcuts yet.
  - Input for the other backends: there is no tap for eframe, iced, Slint or
    the terminal.
- Backend harnesses other than WinUI: eframe through `egui_kittest`, ratatui
  through `TestBackend`'s cell buffer, Slint through its testing backend
  (pinned to the exact version), iced through `iced_test`.
- The call site on `send` and `cx.task()`. `#[track_caller]` is on timers,
  some registration and `Task::spawn`, which keeps it only to name a lane's
  two policies in a panic; the trace records no location. GPUI's profiler records where each task was spawned and Rerun puts
  a `Location` on every command; in the trace it would be a jump to source.

