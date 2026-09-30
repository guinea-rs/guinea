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

### The puffin profiler, half done

Done: zones around every page's and layout's `render` (`devtools::Rendering`),
a frame boundary per backend, `Capability::Profiler`, `Command::Profiler`,
`Report::Profiler`, and a `puffin_http` server the plugin starts on request.

Left: devtools' own half - keep the address the application reports, a command
route on the HTTP API, and a page that connects with `puffin_http::Client` and
draws the frames. `puffin_egui` is no use: it is on egui 0.33 and devtools are
on 0.36, so the flame graph is ours to draw.

Also worth doing: let devtools profile themselves, which is the same page
reading the in-process `GlobalProfiler` instead of a socket.

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
- what a scope will tear down (`own` / `Teardown`);
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
  registers a type in `guinea_core::remote`; `guinea::devtools::act` sends it
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
- The call site on `send` and `spawn_bg`. `#[track_caller]` is on timers and
  some registration only; guinea-core's `send` and `spawn_bg` record no
  location. GPUI's profiler records where each task was spawned and Rerun puts
  a `Location` on every command; in the trace it would be a jump to source.

