---
title: Global shortcuts
sidebar:
  order: 13
---

A global shortcut can show a hidden window or send a command while another
application has focus. Enable the `global-hotkeys` Cargo feature and import
the types from `guinea::hotkeys`; no separate keyboard library is needed.

Register it in the feature that handles the command. `cx.global_hotkey`
takes a binding and a function that builds the feature's message. The
message goes to the handler registered in the same feature.

## A shortcut in settings

Here `setting` is a reactive `Option<HotKey>` implementing
`guinea::sources::Changing`. With amethystate, use the field's `.changing()`
adapter from `amethystate_guinea::IntoChanging`. Opening and storing the
settings remains the application's responsibility.

<!-- shown: hotkeys: binding -->
```rust
struct TogglePanel {
    shortcut: HotKey,
    state: HotKeyState,
}

let binding = cx.global_hotkey(HotkeyBinding::follows(setting.clone()), |event| {
    TogglePanel {
        shortcut: event.shortcut,
        state: event.state,
    }
})?;
```
<!-- /shown -->

`HotKey`, `Modifiers`, `Code`, and `HotKeyState` are re-exported from
`guinea::hotkeys`. `HotKey` supports serde, so the same type can be kept in
settings. A fixed binding can be passed directly as a `HotKey`.

The event contains the effective shortcut and whether it was pressed or
released. Its modifiers describe the registered combination; they do not
report additional keys held by the user. For a toggle, handle only
`HotKeyState::Pressed`.

## Rebinding and conflicts

Changing the setting registers the new combination before releasing the
old one. If the new shortcut is occupied, the old one continues working.
Setting it to `None` releases the reservation.

The returned `binding.status` is a `Feed<HotkeyStatus>`. It contains
`requested`, `effective`, and `error`: the requested shortcut can differ
from the effective one after a conflict. Follow the feed to display that
error in a shortcut editor. An initial failure returns through `?`; a
later failure is published in the feed.

For a shortcut editor, use `HotkeyBinding::follows(setting).recovering()`.
An initial registration conflict then leaves an inactive binding with
`status.error`, allowing the application to start. Changing the setting retries
registration without replacing the stored value with `None`. Errors initializing
the native hotkey manager still return through `?`.

Events already captured for an earlier registration are discarded after a
successful change, including when a shortcut changes away and back again.

## Lifetime

The scope owns the registration. Dropping the returned handle does not
unregister it; removing the scope unregisters the shortcut and drops its
settings subscription and handler. A sleeping scope keeps its reservation
but discards events, with no replay when it wakes.

Register a shortcut that opens a hidden panel in an application feature,
so it remains available independently of the panel's page. Native window
policies and `WindowVisuals` do not own global shortcuts.

This integration uses the toolkit's native event loop. It supports Windows,
macOS on the main thread, and Linux/X11. Wayland portals and shortcuts local
to a focused control are separate capabilities. Guinea owns the underlying
library's event handler; do not replace it or consume its receiver yourself.
