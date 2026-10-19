# Working on guinea

The application layer for Rust GUI. This file is the map; it does not repeat
what the sources or the book already say.

## Where the documentation lives

**Start with the book, not the sources.** It carries what the API reference
cannot: what the pieces are for, how they fit, and why they are shaped the way
they are. It is markdown in this repository:

[`landing/src/content/docs/introduction.md`](landing/src/content/docs/introduction.md)

Per-item reference is rustdoc: `cargo doc --open` for the working tree.
`TODO.md` holds what is decided and not done, and what is not decided yet.

## Layout

```
crates/guinea-core       scopes, actors, the bus, state, observability
crates/guinea-app        features, plugins, services, the host and its roots, the harness
crates/guinea-router     keyed mounting, navigators, slots, deep links
crates/guinea-macros     #[installs], feature!, actor!, #[reducer], #[handler], derive(Routable)
crates/guinea-trace      the causal trace and the JSON log layer
crates/guinea-mark       names for what tests and tools reach for on screen
crates/guinea-meta*      app.toml read at build time, its constants at run time
crates/guinea-codegen    for build.rs: inputs read and watched, files written only when they change
crates/adapters/*        backends: winui (the main one), iced, eframe, ratatui, slint
crates/guinea            the facade users depend on: re-exports and the tests across crates
crates/vendor/*          pre-release windows-rs crates the WinUI backend builds on
examples/                applications, each its own workspace
landing/                 the landing page and the book (Astro + Starlight)
xtask/                   cargo xtask docs
```

Plugins - settings, window geometry, localisation, devtools - live in
[guinea-plugins](https://github.com/guinea-rs/guinea-plugins).

## Checks

What CI runs, and what a change should pass before it is pushed:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo xtask docs --check
cargo build -p guinea --no-default-features
```

## Working on the book

Two locales over the same tree: English at `landing/src/content/docs/`, Russian
under `ru/` beside it. A page added to one belongs in the other, and a change
to one lands with the change to the other.

**Code in the book is not written in the book.** A page asks for a block by
name:

```markdown
<!-- shown: introduction: a feature -->
<!-- /shown -->
```

and `cargo xtask docs` fills it from the test that marked the same name:

```rust
//@show introduction: a feature
...
//@show-end
```

A test made for a page is `book_<page>.rs` in the crate whose API it shows.
Both locales draw from the same marks, so the code on the two pages cannot
drift apart; only the prose is translated. `//@hide` .. `//@unhide` is what a
block runs and does not show. The grammar is the
[`shown`](https://github.com/guinea-rs/shown) crate's.

**To change an example: change the test, run it, run `cargo xtask docs`.**
Editing the fenced block in the page is wasted work; the next run overwrites
it, and `--check` fails CI until it does.

Diagrams are Mermaid fences, not ASCII art: an agent reads a graph from text
reliably and a two-dimensional picture of one badly. ASCII is for trees.

To see the book: `npm --prefix landing run dev`, then
<http://localhost:4321/guinea/introduction/>.
