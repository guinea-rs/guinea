# NavigationView selection-indicator scratchpad

Reported symptom: the native selection-indicator animation stops working after
navigation or later updates. It is not known to be tied to the first navigation.
This is an investigation fixture, not a confirmed reproduction or a fix.

The same menu is rendered by two hosts. Both use published Guinea 0.35.1 and
windows-reactor-pre 0.107.0, matching the inspected Uniproc binary. No vendor
changes, Uniproc service, profile recording, external assets or settings files
are involved.

From the Guinea repository:

```sh
cargo run --locked --manifest-path scratchpads/navigation-indicator/Cargo.toml
cargo run --locked --manifest-path scratchpads/navigation-indicator/Cargo.toml -- --guinea
```

The default host is a plain Reactor component. `--guinea` uses a Guinea layout,
navigator, outlet and four separately mounted pages. The menu declarations are
shared: stable keyed items, explicit `is_selected`, and Settings in the footer.

Change one variable at a time (close the previous fixture before the next run):

| Switch | Purpose |
| --- | --- |
| `--uncontrolled` | Omit every item's `is_selected`; native selection owns the indicator. Initial selection is unset. Programmatic Next is disabled. |
| `--static-content` | Keep content fixed while selection changes; Guinea does not mount an outlet in this variant. |
| `--no-icons` | Omit icons. Otherwise use constant FontIcon glyphs, without SVG processing. |
| `--tick` | Raw Reactor only: trigger an update every second without changing selection. |
| `--defer-selection` | Guinea only: send the native click as a layout message before calling the navigator, instead of navigating directly inside the selection callback. |

Observation from the user: programmatic Next animates in both hosts (although
the motion looks poor); manual item selection breaks animation only in Guinea.
The earlier interpretation that both hosts failed was corrected. The
`--defer-selection` variant isolates event-delivery timing; it is not yet a fix.

The user then confirmed that `--guinea --defer-selection` preserves the animation
on manual clicks. This variant keeps the same NavigationView declarations,
controlled selection, route changes and outlet. It only sends a layout message
before invoking the navigator, instead of invoking it in the selection callback.
This establishes an event-delivery boundary that changes the symptom. It does not
yet distinguish dispatch ordering, callback identity/rebinding, or native
animation reentrancy as the underlying mechanism.

Navigator source inspection shows that `commit` changes the current route before
notifying watchers; the hypothesis that current() simply stays old until outlet
mounting is not supported. The deferred run logged two layout renders per change,
both with the new selection, so a second render alone is not sufficient evidence
of the defect. Do not require consumers to add message forwarding as a final fix.

In either mode, **Redraw same selection** updates unrelated footer text;
**Next (programmatic)** changes the controlled selection through state/navigation.
Manually select adjacent items, nonadjacent items and Settings. Compare the
indicator before and after unrelated redraws and content replacement. Record
which combination actually fails; a build or static screenshot does not prove
that an animation ran.

## Findings before this fixture

- Uniproc's menu uses stable `nav.tag()` keys and a stable NavigationView position
  in the outer Grid.
- Reactor's keyed-relation fast path retains items when type/key/order match;
  there is no unconditional collection clear in that path.
- Reactor records native selection feedback in retained item selection flags;
  this is intended to avoid echoing selection changes on the next render.
- The inspected application resource overrides change brushes and icon-box
  height, not animation timing or selection-indicator storyboards.
- One read-only screenshot of the running application was inspected. No click,
  activation, live animation measurement or causal reproduction was performed.

These facts rule out neither a later native mutation nor a WinUI animation-state
bug. Do not report either as established without reproducing it.

Upstream animation implementation:
https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/NavigationView/NavigationView.cpp

The source requires previous/next indicator elements, a template, and animations
enabled. It also handles interrupted animations and delayed item realization;
the inspected upstream main branch is context, not proof about the installed DLL.

## Confirmed cause and correction

The native recording establishes the following sequence on a manual click:

1. WinUI selects Activity (object 16).
2. Chrome receives the navigator's route-change notification before the layout.
3. Reactor expands Chrome using the layout's cached View. That View still declares
   Processes selected. The batch from component 1 writes object 14 true and
   object 16 false.
4. The layout's own refresh runs. Component 2 writes object 14 false and object
   16 true. The selection has been reversed and restored during the animation.

The same reversal was captured for Activity -> Services. Thus it is not tied to
the first click, icons, item replacement, or outlet mounting failure. Programmatic
navigation starts with the old native selection, so replaying that cached View
does not cause the same reversal. Deferring the click through a layout message
works because that layout renders its new selection before the ancestor refresh.

Chrome needs navigator notifications to show/dismiss a guard question, not to
render every route change. `question_watch::watch_questions` now notifies Chrome
only when `pending()` changes. The layout keeps its normal navigation subscription.
No consumer messages, vendor edits, or animation overrides are required.

The control used the same instrumented Reactor and a copy of published
guinea-winui 0.35.1 with only this observer change. Manual clicks then produced
native selection observations and layout renders with no intervening IsSelected
writes. The user confirmed that the fixed window looks normal.

Diagnostic outputs (local, not published):

- `target/nav-traced-direct.log`: native selection followed by Chrome reversal
  and layout restoration.
- `target/nav-traced-fixed.log`: native selection and layout render, no reversal.
- `target/navigation-indicator-reactor-traced`: instrumented copy of the registry
  Reactor, separate from the repository's unchanged vendor.
- `target/navigation-indicator-guinea-fixed`: registry adapter with the observer
  correction only, separate from unrelated development changes.

Regression coverage checks that ordinary route changes do not refresh Chrome,
guard questions still open and close on rejection/acceptance, and dropping the
watch removes its notification. The runtime mechanism of cached-child replay is
broader than this adapter's unnecessary subscription; this correction removes the
confirmed Guinea trigger rather than changing Reactor's component semantics.
