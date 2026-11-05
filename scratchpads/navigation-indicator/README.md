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
