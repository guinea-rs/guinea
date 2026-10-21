<div align="center">

<img src="https://raw.githubusercontent.com/guinea-rs/guinea/master/crates/guinea/assets/banner.svg" alt="guinea" width="100%" />

*The application layer for Rust GUI.*

</div>

[Read the book](https://guinea-rs.github.io/guinea/introduction/) ·
[Читать книгу на русском](https://guinea-rs.github.io/guinea/ru/introduction/)

[Todo application on five backends](https://github.com/guinea-rs/guinea/tree/master/examples)

### Why

Rust has good toolkits and no application layer above them: a toolkit draws the screen,
and how the application around it is built is left to each project. The answers differ,
but every application past a demo meets the same problems:

- messages forwarded through layers to where they are handled;
- request ids, so a late answer does not overwrite a newer one;
- setup, teardown and refetching written by hand for every screen;
- one huge state everything reaches into, and no tests for it;
- settings saved and synced by hand.

### What guinea is

MVI for a whole application:

- **Intent** - a page emits what the user asked for, and changes only its own view state;
- **Model** - a reducer, the one place a state changes, driven by an actor that decides;
- **View** - the page, written the way its toolkit works: WinUI, iced, egui, ratatui or Slint.

And the application is built out of pages. Everything lives in a scope - the
application's, a window's, a layout's, a page's - and goes with it: what a page set up
ends when the user leaves it, with nothing to tear down by hand. Nothing pushes you
towards global state: state lives with the screen that needs it, and is shared by
installing it higher up - in a layout, or in the application. A page reads what it
installed itself, what a layout above it exports, and what the application provides;
anything else is not in reach, and a test sets up exactly that and no more.
Settings go to [amethystate](https://github.com/guinea-rs/amethystate).

### Architecture

<img src="https://raw.githubusercontent.com/guinea-rs/guinea/master/crates/guinea/assets/architecture.svg" alt="Guinea architecture: GUI integration, application composition, scoped feature logic, testing and observability, connected to application-specific behavior" width="100%" />

Toolkits draw the interface; guinea connects it to scope-owned application logic.
Plugins and features declare what they need and provide. Scopes determine where
those values are visible and when their resources are released.

What an application needs besides its pages - a settings store, the window's geometry
remembered, localisation, a single instance, devtools - comes as
[plugins](https://github.com/guinea-rs/guinea-plugins).

guinea takes its ideas from Kotlin, Swift, gpui and Next.js. Its ownership is the one
Leptos and Dioxus arrived at - a tree of scopes owning `Copy` handles - put under
toolkits that do not have one.

> [!WARNING]
> **guinea is young, and its API still moves between minor versions.**
> WinUI is the backend a real application ([uniproc](https://github.com/uniproc-dev/uniproc)) runs on every day; the other four
> run the same example application and are tested, but nobody depends on them yet.
