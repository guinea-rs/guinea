## windows-reactor

Windows Reactor is a typed declarative UI library for building native WinUI 3 applications in
Rust. Components own Rust state, describe the current view, and receive typed messages from
controls.

Add Reactor to `Cargo.toml`:

```toml
[dependencies.windows-reactor]
version = "0.100"
```

```rust,no_run
use windows_reactor::*;

#[derive(Clone, Copy)]
enum Message {
    Increment,
    Reset,
}

struct Counter {
    count: i32,
}

impl Component for Counter {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), _context: &ComponentContext<Self>) -> Self {
        Self { count: 0 }
    }

    fn update(&mut self, message: Message, _context: &ComponentContext<Self>) {
        match message {
            Message::Increment => self.count += 1,
            Message::Reset => self.count = 0,
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let content = StackPanel::new().spacing(8.0).children((
            format!("Count: {}", self.count),
            Button::new()
                .on_click(context.message(Message::Increment))
                .content("Increment"),
            Button::new()
                .on_click(context.message(Message::Reset))
                .content("Reset"),
        ));

        context.window_frame("Counter", content)
    }
}

fn main() {
    App::run_component::<Counter>(()).unwrap();
}
```

Run with `cargo run`. Applications use the default features and an installed Windows App SDK
runtime. See the [setup guide][setup] for runtime prerequisites and self-contained deployment.

The [getting-started guide][guide] covers editable state, child components, background work, images,
and application windows. Browse the [samples] for complete applications and focused examples.

[setup]: https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-reactor-setup.md
[guide]: https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-reactor.md
[samples]: https://github.com/microsoft/windows-rs/tree/master/crates/samples/reactor

## Native scrolling integration

This pre-release adds `ElementRef<ScrollViewer>::observe_scroll_viewer` and
`request_change_view`. Keep the returned `ElementObservation` alive. Observations
include an initial snapshot (possibly before layout), offset changes and layout-only
changes to viewport or extent; identical snapshots are suppressed.

`request_change_view(vertical_offset, disable_animation, completion)` returns whether
the command was queued. Its completion receives the native `Result<bool,
IntegrationError>` acceptance result, not animation completion. Detached or superseded
bindings report `Unavailable`; requests that could not be queued return `false`
without calling completion.

For a composition-rendered virtual list, put a full-height `Grid` inside the native
`ScrollViewer`, attach the child visual to that grid, and place realized rows in
content coordinates. The native control scrolls the content; do not subtract the
offset a second time. Metrics select the realized range, not the animation curve.

The supplemental native projection is generated from Windows App SDK metadata with
`windows-bindgen` 0.100 using `input_default`, `minimal`, `dead_code`, `flat` and
`src/native/scroll_bindings.txt` as `filter_file`. It exposes no raw native objects
through the public scrolling API.

Lifecycle tests run with the crate's regular tests. The native layout test opens
a window and runs separately on an interactive Windows desktop:

```text
cargo test --manifest-path crates/vendor/windows-reactor-pre/Cargo.toml --features test native_scroll_viewer_moves_full_height_content_and_reports_layout_changes -- --ignored --test-threads=1
```
