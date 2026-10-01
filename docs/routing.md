# Routing

**Status: design.** None of this is implemented yet. The current `routes!`
still declares a path string on every page and hands `install` an `AppUri`.
This document records what the declaration should become and why, so the work
can be split without re-deriving the reasoning.

## The thesis

A route is a typed value. The enum `routes!` generates *is* the model, and
`navigate(Route::Processes { context })` is the only way to move.

Strings are a boundary format. There is no address bar on a desktop, so a path
is needed at exactly two boundaries: a deep link arriving from outside, and a
session restored after a restart. Both are opt-in per route, because a
framework cannot decide whether an application wants every screen addressable
or three of them.

Everything else follows from taking that literally.

## Why the current shape has to change

The string is not at the boundary today - it is on the hot path. Every
navigation serialises the typed route and every feature parses it back:

```rust
router.navigate(initial.clone(), &initial.to_uri())?;
uri.segment(0).expect("route always carries a :context segment")
```

A `context: String` field exists on the enum, is encoded into a path, and is
then dug back out by index. Five defects live on that road, and all five stop
being possible once the road is gone rather than being fixed one by one:

| | |
|---|---|
| `to_uri()` panics on a space | `PathAndQuery` rejects `"/a b/processes"`; the `expect` claims this cannot happen |
| a failed capture parse ends matching | `parts[i].parse().ok()?` returns from `parse`, so later arms are never tried |
| declaration order decides matches | a leading capture shadows any same-length pattern declared below it |
| an empty parameter breaks round-trip | `path()` emits `//processes`, `parse` drops empty segments, lengths disagree |
| no percent coding in either direction | `%20` reaches the actor as `%20` |

Fields and captures are also never cross-checked; a mismatch surfaces as an
unresolved identifier inside generated code.

## The declaration

```rust
routes! {
    backend = guinea::slint::Slint,

    Route {
        layout(TabsLayout) restorable {
            page(Processes) { context: String }
            page(Metrics)   link("/m/:context") { context: String }
        }

        page(Secrets) { context: String, token: ~Session }

        layout(AdminArea) guard(RequiresAdmin) {
            page(Audit) { }
        }

        page(Splash)
    }
}
```

Every node reads the same way: **kind, type, modifiers, body**. The body is a
brace group and it terminates the node, so modifiers are read until the first
brace group, the next node keyword, or the end - no lookahead.

`layout(T) { .. }` wraps its children. It nests freely and sits beside pages at
any level. It declares no fields: a layout's parameters are derived as the
intersection of its descendants', because a layout can only rely on what all of
its children carry. There is nothing to keep in sync.

A layout may appear more than once, but always under the same layouts: what it
may read from above is checked against one answer, and a second place with
other ancestors would compile against the first and find nothing in the second.
The macro refuses such a tree.

Each pair of parentheses holds one thing: `page(T)`, `layout(T)`, `guard(G)`,
`link("...")`. A page's address follows it as `link`, never inside it, and two
guards are two `guard(..)`s. Anything more in the parentheses is an error, not
something quietly dropped.

`page(T) { .. }` is a leaf, and its fields are the route's parameters. They
reach `install` typed. The body is optional.

Nesting and addressing are independent. The tree says what wraps what; `link`
says what is reachable from outside. A page needing its own chrome is declared
beside the layout it is escaping, not inside it with an override - which is why
none of the route-group machinery other frameworks grew (`(folder)` in Next,
`+page@` in SvelteKit) has an equivalent here. That machinery exists to break a
folder-to-URL coupling we never had.

## Reachability

Three tiers, each opt-in, each imposing its bounds only where it is asked for:

| | what it means | what it requires of fields |
|---|---|---|
| *(default)* | reachable only from inside, through `navigate` | nothing |
| `link("...")` | reachable from outside | every field is a capture, and its type encodes as a segment |
| `restorable` | survives a restart | every field is `Serialize + Deserialize` |

That `link` requires *every* field to be a capture is deliberate: an incoming
link has to reconstruct the destination completely. It also pushes the design
the right way - an `Arc<Feed>` in a route parameter is a dependency, not an
identity, and belongs in the container.

A route that opted into nothing may carry anything. That is the escape hatch
for the genuinely local case: a wizard step holding a live result, a modal
holding a callback.

This is the mobile lesson taken precisely. Android forces every route key to be
serialisable because the system kills processes; the constraint then leaks into
every route argument. Ours appears only where the feature is used.

## The cascade rule

> Cascade what tightens. Do not cascade what opens.

- `guard` tightens - it cascades, declared once for an area.
- `restorable` demands proof from the compiler - it cascades, and failures land
  on the offending field.
- `link` opens an external entry point - it is flat and explicit, never
  inherited.
- `keep` neither tightens nor opens: it says what becomes of one segment when a
  navigation leaves it, so it is flat. See [Keeping a segment](#keeping-a-segment).

A cascading `link` would make renaming `/app` to `/workspace` - a wide breaking
change to a published contract - look like a one-word edit. Flat paths make it
five lines of diff, each reviewed. Repetition costs little: an application has
a handful of external entries, and the shared part usually lives in the scheme.

`link` on a layout is an error. A layout is never a leaf, so the address would
have to resolve onward to some default page, and implicit onward resolution is
what breaks history everywhere it is tried. An area's entry point is an
explicit page:

```rust
layout(TabsLayout) {
    page(Overview) link("/app") { context: String }
    page(Processes) { context: String }
}
```

Opting out of an inherited guard must name it - `!guard(RequiresAdmin)`, never
a bare negation - so removing protection reads as removing protection.

Guards are told apart by their whole type as written. `HasRole<Admin>` and
`HasRole<Guest>` are two guards, so `!guard(HasRole<Guest>)` does not open an
area that `HasRole<Admin>` closes; `auth::Session` and `legacy::Session` are two
guards as well. An opt-out spells the guard the way the layout above it did.

## Identity and payload

The router asks one question about a route: is this the same route, or the same
page with different parameters? Today it answers by storing a clone and
comparing, which forces `Clone + PartialEq + Debug` on every field.

Those bounds are wrong for the payload a free route is allowed to carry. A
channel has no identity; a callback has none; `Arc<dyn Trait>` has only an
address.

So the question is answered by generated code instead of by trait bounds. The
macro emits an identity value per route, the router stores that, and the route
type itself needs nothing. `Debug` is generated by hand over the identity
fields, so tracing stays useful without demanding anything of the payload.

A field is part of the identity unless marked `~`:

```rust
page(Wizard) { step: u8, result: ~Receiver<Report> }
```

A `~` field never participates in comparison and is never stored, so a page
carrying one reinstalls on every entry - correct, since a new channel is a new
thing. `~` reads as "approximately": a field for which "the same" is not a
meaningful question. The token is vacant in Rust - it meant `~T` before 1.0 and
was never reused - and it lexes cleanly inside a macro body.

Because both outward tiers must reconstruct a route whole:

> `~` is allowed only on a route that is neither `link` nor `restorable`.

## Keeping a segment

A navigation tears down every segment it leaves. For most that is right - a
page's state is the page's, and coming back to it is coming to it fresh. An
area whose state is expensive to have again is the exception: a process list
that starts from `Loading` until the next report, with its sort and selection
gone, because the user looked at another tab for a second.

Moving the feature up into the shell keeps it, and leaks it into every page
under the shell. `keep` keeps it where it is:

```rust
layout(ProcessesArea) keep {
    page(Processes)
    page(ProcessesSettings)
}
```

Left, a `keep` segment sleeps instead of going. Its scope stays, with its
reducers, its actors and its features; coming back to it wakes the same scope -
no `install`, no `init` - and the view draws from the state it kept.

Asleep, it is deaf. What its scope owns stops: its timers skip their ticks, and
its actors and its callbacks are not told what either bus carries. Nothing is
queued for it, so what happened meanwhile is missed rather than replayed. A
feature catches up when it wakes:

```rust
cx.on_wake(move || agent.send(AgentStateRequest));
```

Work a feature started on its own - a task it spawned, a stream it pours into a
reducer - runs on: guinea cannot pause what it did not start. Such work can ask
`cx.scope.awake()` whether there is anyone to do it for.

What keeping does not change:

- A segment that sleeps is not leaving, so its leave guards are not asked.
- It wakes only where it slept: under the very scopes it was installed under,
  with the same capture. Reached with another capture, it is torn down and
  installed fresh; when a segment above it goes, it goes first.
- `keep` is not inherited, and a `keep` segment under one that a navigation
  tears down goes with it - there is nothing left to wake it under.
- It is runtime only. Nothing about it survives a restart; `restorable` is a
  separate promise.

There is no time-to-live. A sleeping segment costs memory and nothing else, and
a window closing takes everything asleep in it down, innermost first.

## Guards

A guard is a first-class stage of navigation, not a redirect. Redirect-as-guard
corrupts history, and both mobile ecosystems and the web reached that
conclusion independently; the one implementation that got it right is Prism's
`IConfirmNavigationRequest`, a genuinely cancellable step with a continuation.

**Entering and leaving are not symmetric, and the asymmetry decides where each
is declared.** On leaving, the scope exists, so the guard can read its own
reducer state - that is the whole point of "unsaved changes". On entering, the
scope does not exist yet, because the guard must answer before anything is
installed. So leave guards register during `install`, and enter guards live in
the route declaration.

Navigation therefore splits into two phases: **decide**, which mutates nothing,
and **commit**, which is today's `install_from` unchanged. Guards run in the
first. Nothing may be torn down or installed until every guard has answered -
otherwise a superseded navigation leaves a half-built tree.

Order mirrors the two directions: leaving runs innermost first, because the leaf
holds the unsaved form; entering runs outermost first, because an area's
authorisation should refuse before an inner page considers anything.

### Optionally async, without taxing the common case

A guard returns a value, not a future. The function stays synchronous:

```rust
pub enum Verdict {
    Allow,
    Block,
    Ask(Decision),
}
```

`Allow` allocates nothing, so the overwhelming majority of navigations stay
exactly as synchronous as they are now. "Optionally async" is the right to
return the third variant.

`Decision` is a token, not a future. There is no `LocalSet` in the tree, the
future would be `!Send`, and this is an actor system rather than a combinator
one: the guard hands the token wherever the answer will come from, and
`decision.allow()` or `decision.block()` resolves it. Sugar covers the common
shapes; `on_leave` is the escape hatch:

```rust
ctx.block_leave_while::<Editor>(|s| s.dirty);
ctx.confirm_leave::<Editor>(|s| s.dirty, l10n::unsaved());
```

### A parked navigation is state, not a call

A pending question lives on the router, as plain data - text, two labels, and
the `Decision`. Nothing toolkit-specific, in keeping with the `Ui` seam.

Every backend can draw it; none of them lacks the capability. Ratatui and eframe
draw it over the frame and **must swallow input while it is up**, or tabs keep
switching underneath the dialog - that obligation belongs in the adapter
contract explicitly, because it is easy to forget. WinUI keeps a
`ContentDialog` beside the route tree, open while a question is pending, and
its buttons answer it. Slint's window is the application's own `.slint`, so
guinea cannot draw into it: `guinea_slint::on_question` is told when a question
comes and goes, and whatever the window shows answers through
`guinea_slint::answer`. Headless hands it to the test, which makes guards the
one part of the router fully testable without a backend.

A backend that draws only when told to learns of a question through
`Router::on_question`; one that draws every frame reads `Router::pending` as it
goes.

Because the question is router state, `Navigation::Deferred` and "a question is
pending" are the same condition, not two.

### Supersession

A second navigation while a guard is pending supersedes the first. The stale
`Decision`'s verdict is discarded by generation counter. Last intent wins, which
is what a user expects, and it is safe precisely because the decide phase
mutated nothing. Prism's callback API leaves this hole open.

## What the macro generates

```rust
enum Route { Processes { context: String }, ... }

struct ProcessesParams { context: String }
impl Page for Processes { type Params = ProcessesParams; }

impl Route {
    fn identity(&self) -> RouteIdentity;
    fn link(&self) -> Option<String>;
    fn from_link(s: &str) -> Option<Route>;
    fn deep_links() -> &'static [&'static str];
}

impl RouteChain<Backend> for Route { ... }
```

`SegmentEntry.install` takes `&dyn Any` and the generated glue narrows it to
`Self::Params`. The author never sees `Any`.

Checks the macro can make, because the tree is static:

- a capture names a field that exists, and whose type encodes as a segment;
- two `link`s of the same shape are an error, so declaration order stops
  mattering;
- matching is built as a prefix tree with literals tried before captures, so a
  failed decode means "this branch did not match" rather than "matching is
  over";
- `link` on a layout, `~` on a route that reaches outside, and a `!guard` naming
  a guard that was never inherited are all errors.

## The deep-link manifest

The full external surface is derivable, so it should be written down and
diffed. What bites about a deep link is not that it is malformed but that it
*changed*: once shipped, the string lives in shortcuts, mails, and other
applications' integrations.

Build scripts run before `rustc`, so nothing built at that stage can see types
or macro expansion. The Slint route tree has to exist before compilation and
therefore has to be read out of `src/routes.rs` as text - that is a hack, and
it is the least bad one available, so the path should at least be named in
`build.rs` rather than hard-coded in a constant inside another crate.

The manifest has no such constraint. It is needed by the installer and by
review, not by the build, so it is generated **after** compilation from
`Route::deep_links()` - the real tree, already type-checked, with no second
parse to disagree with the first.

Its consumers are three, and none depends on the others: the compiler checks
the form, the committed manifest guards compatibility, and the installer
registers the schemes with the OS. Registration is a fact about the
application, so it belongs with the rest of them in `guinea-meta`.

The surface should list guards alongside links: what is reachable from outside
and what stands in front of it is one review, not two. External activation
carries no caller identity - Microsoft's own URI documentation says any process
including malware can launch a scheme - so a new external entry must be
readable as an added line in a diff.

## Rejected, and why

**Query parameters.** Query solves "the URL is the application's state
container", which is a consequence of the browser owning navigation. Deep links
arriving from outside carry identity, not view state; filters and sort orders
belong in a reducer, and in the store if they must outlive a run. The rule that
every field of a `link` route is a capture already pushes back on encoding view
state into a path: it would make the filter part of the route's identity and
reinstall the page on every change.

**Path composition between pages** (`page(Print, Processes / "print")`). It
solved drift between two strings. Once a path exists only on routes that opted
in, most pages have no string at all and there is nothing to drift.

**The declaration as a non-Rust file.** The tree names Rust types with full
paths and generics; a file would turn them back into strings with nothing to
check them, which is the disease this whole design removes. A build script
cannot help - it runs before the crate compiles, so it can never see types. The
only way to give a build script real types is a separate crate compiled first,
and the tree names types from the application itself, so that is circular.

**`Serialize` on every route.** Only `restorable` asks for it.

## Open

- `Verdict::Redirect`. Without it, "not authorised, go to login" gets
  reimplemented as a redirect and the original bug returns. With it, the route
  is generic and has to be erased the way `prev_route` already is.
- Whether the address surface is also authored as a file rather than derived.
  Smaller than it looked, and safely deferred: addresses have to exist as a
  concept first.
- Names for the leave-guard sugar.
