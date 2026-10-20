---
title: Pages and layouts
sidebar:
  order: 2
---

A page connects features to the interface: it installs the features it needs,
reads their state and sends actions in response to input. How it describes
the interface depends on the toolkit.

This chapter implements the same counter on WinUI and egui. The tabs let
you compare their code. Feature installation, state access and page
lifetimes follow the same rules on both backends.

## A page

A page implements `Page`. Its struct holds the view's own data: here,
`step` is the amount the button will add. The counter's value belongs to the
`Counter` feature from the [introduction](../../introduction/).

::::backends
:::winui
<!-- shown: winui: a page -->
```rust
use guinea::winui::reactor::{Button, StackPanel, TextBlock, View};
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};

#[derive(guinea::Mark)]
pub enum Marks {
    Add,
    Bigger,
}

pub struct CounterPage {
    step: u32,
}

pub enum Msg {
    Bigger,
}

#[page]
impl Page for CounterPage {
    type Message = Msg;

    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Counter>(&())?;
        Ok(Self { step: 1 })
    }

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::Bigger => self.step += 1,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (count, dispatch) = cx.read::<Count>();
        let step = self.step;

        StackPanel::new()
            .children((
                TextBlock::new().text(format!("{} by {step}", count.0)),
                Button::new()
                    .content(TextBlock::new().text("Add"))
                    .on_click(move || dispatch.emit(Add(step)))
                    .mark(Marks::Add),
                Button::new()
                    .content(TextBlock::new().text("Bigger"))
                    .on_click(cx.on(|()| Msg::Bigger))
                    .mark(Marks::Bigger),
            ))
            .into()
    }
}
```
<!-- /shown -->

On WinUI, describing the interface and handling messages are separate:

- **`view`** builds a `View` describing the interface. `cx.read::<Count>()`
  returns the state and a `Dispatch` for its owning feature. It also
  subscribes the page component to `Count`: when the state changes, guinea
  requests a new view and Reactor updates the element tree.
- **`update`** changes the page's own state. `cx.on(|()| Msg::Bigger)` turns a
  button event into `Msg::Bigger`. The handler increases `step`, and the
  component calls `view` again.

The `Add` button sends its action straight to the feature. Its handler
outlives the call to `view`, so the closure captures `dispatch` and `step`
with `move`. Both types implement `Copy`, so the values are copied into it.
:::
:::egui
<!-- shown: egui: a page -->
```rust
use guinea::eframe::{Page, PageCx, page};

pub struct CounterPage {
    step: u32,
}

#[page]
impl Page for CounterPage {
    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Counter>(&())?;
        Ok(Self { step: 1 })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (count, dispatch) = cx.read::<Count>();
        let ui = cx.ui();

        ui.label(format!("{} by {}", count.0, self.step));
        if ui.button("Add").clicked() {
            dispatch.emit(Add(self.step));
        }
        if ui.button("Bigger").clicked() {
            self.step += 1;
        }
    }
}
```
<!-- /shown -->

On egui, **`render`** both builds the interface and handles input. It takes
`&mut self`, so it can change the page's fields directly. `clicked()` reports
a button click, which the code handles in that call. No separate page
messages or `update` method are needed.

`cx.read::<Count>()` returns the state and a `Dispatch`. Unlike WinUI, the
read does not subscribe a component: `render` reads the current value again
when building the next frame.
:::
::::

**`install`** creates the page instance when it is first mounted. Here it
installs `Counter` and returns a `CounterPage` whose initial step is `1`.
Subsequent redraws do not repeat installation. The feature's resources live
in the page's scope; see [Scopes](../scopes/).

The two buttons illustrate the distinction between page and feature state.
`Bigger` changes the page's `step` field. `Add` sends `Add(step)` to the
feature, which handles the action and updates `Count`.

`#[page]` fills in optional parts of the implementation. With no `install`,
it creates the page through `Default`. With no declared `Params`, it uses
`()`.

## Dependencies during installation

`#[page]` and `#[layout]` also resolve dependencies declared as arguments
after `cx` in `install`, just as `#[installs]` does for a feature. For example,
`fn install(cx: &Install, store: Store) -> anyhow::Result<Self>` receives a
clone of the `Store` registered with `app.provide(store)`. There is no need
to call `cx.service::<Store>()` manually.

A plain `Store` argument is required: if it was not provided, installation
returns an error before entering the method body. An `Option<Store>` argument
is optional and receives `None` when the service is absent. Apply a fallback
explicitly in the body if needed, for example with `unwrap_or_default()`.

These arguments do not replace the page's or layout's `Params`: values passed
when mounting remain available through `cx.params()`. Resolving a service does
not install a feature automatically; the body still calls `cx.install` for
the features the page or layout needs.

## A layout

A layout implements `Layout`. It places shared interface elements and
mounts child pages or layouts.

Here `Shell` shows a navigation button and one of two pages. The `Place`
enum describes the route. `cx.outlet` supplies the current route, and the
layout's `match` chooses what to mount.

::::backends
:::winui
<!-- shown: winui: a layout -->
```rust
#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Place {
    Counter,
    About,
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Place>();
        let to_about = nav.clone();

        StackPanel::new()
            .children((
                Button::new()
                    .content(TextBlock::new().text("About"))
                    .on_click(move || {
                        let _ = to_about.to(Place::About);
                    }),
                cx.outlet(&nav, |place, cx| match place {
                    Place::Counter => cx.mount::<CounterPage>(&()),
                    Place::About => cx.mount::<About>(&()),
                }),
            ))
            .into()
    }
}
```
<!-- /shown -->
:::
:::egui
<!-- shown: egui: a layout -->
```rust
#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Place {
    Counter,
    About,
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Place>();
        if cx.ui().button("About").clicked() {
            let _ = nav.to(Place::About);
        }

        let page = cx.outlet(&nav, |place, cx| match place {
            Place::Counter => cx.mount::<CounterPage>(&()),
            Place::About => cx.mount::<About>(&()),
        });
        page.draw(cx.ui());
    }
}
```
<!-- /shown -->

On egui, `cx.outlet` returns a `Node` containing a deferred drawing call.
`draw` executes it in the supplied `ui`, letting the layout choose where
its child content appears.
:::
::::

`cx.navigate::<Place>()` looks for a navigator of that type in the current
scope and its ancestors. Here the window creates it. Calling
`nav.to(Place::About)` requests a route change. Once allowed, the next
content build selects `About` through the outlet. `mount` reuses a page with
the same type and equal parameters, or installs a new instance. Children no longer mounted
are removed; those marked `keep` sleep instead.

A layout's `install` works like a page's. Features installed in the layout
can export state to its child pages.

## An application

The `app` function receives a `FeatureBuilder`. It installs shared features
and plugins, provides application data and returns a declaration for the
first window. This example has no shared dependencies, so it only creates
the `Window`:

::::backends
:::winui
<!-- shown: winui: an application -->
```rust
use guinea::winui::Window;

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}
```
<!-- /shown -->
:::
:::egui
<!-- shown: egui: an application -->
```rust
use guinea::eframe::Window;

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}
```
<!-- /shown -->
:::
::::

`Window::of::<Shell>()` selects the root layout. `.at(..)` configures the
window to open a navigator at the initial route. Features installed with
`app.install` live in the application scope, shared by its windows.

The executable's entry point calls the backend's `run(app)` to open the
window and enter its event loop. The tests below call `app` too, but mount
the returned window without starting an operating-system event loop.

## Testing

A backend harness runs page code without an operating-system window: it
delivers input, runs the work caused by actions and checks the rendered
result. It uses the same domain `Harness` as feature tests.

::::backends
:::winui
<!-- shown: winui: testing a page -->
```rust
#[guinea::test]
fn adds_the_step_it_was_told(h: &mut Harness) {
    let mut page = Mounted::<CounterPage>::mount(h.segment(), ()).unwrap();

    page.click(Marks::Bigger).settle();
    page.click(Marks::Add).settle();
    page.settle();

    assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
}
```
<!-- /shown -->

`#[derive(guinea::Mark)]` and `.mark(..)` name elements for tests. A button's
name can stay the same when its label or position changes. `click_text`
finds an element by its text instead.

`settle` runs the work caused by the click: handling the action, updating
the reducer and redrawing the page. The test seed determines background
execution order so a failing run can be reproduced.

A layout is tested the same way, started at a route:

<!-- shown: winui: testing a layout -->
```rust
#[guinea::test]
fn the_about_button_moves_the_window(h: &mut Harness) {
    let mut shell = Mounted::<Shell>::mount_at(h.segment(), (), Place::Counter).unwrap();
    assert!(shell.find_text("0 by 1").is_some(), "{:#?}", shell.tree());

    shell.click_text("About").settle();
    shell.settle();

    assert_eq!(shell.navigator::<Place>().current(), Some(Place::About));
    assert!(shell.find_text("guinea").is_some(), "{:#?}", shell.tree());
}
```
<!-- /shown -->

`Mounted` is behind the `harness` feature:

```toml
[dev-dependencies]
guinea = { version = "0.31", features = ["harness"] }
```

The window returned by `app` can also be tested directly, including its
initial route:

<!-- shown: winui: testing an application -->
```rust
#[guinea::test]
fn the_application_opens_at_its_declared_route(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut shell = Mounted::<Place>::from_window(h, window).unwrap();
    assert_eq!(shell.route(), Place::Counter);
    assert!(shell.find_text("0 by 1").is_some());

    shell.click_text("About").settle();

    assert_eq!(shell.route(), Place::About);
    assert!(shell.find_text("guinea").is_some());
}
```
<!-- /shown -->
:::
:::egui
The egui harness runs frames without an operating-system window. It delivers
pointer presses and releases to egui and reads the text from the shapes the
page drew:

<!-- shown: egui: testing a page -->
```rust
#[guinea::test]
fn adds_the_step_it_was_told(h: &mut Harness) {
    let mut page = Mounted::window(h, Window::of::<CounterPage>());

    page.click_text("Bigger");
    page.click_text("Add");

    assert!(page.has_text("2 by 2"));
}
```
<!-- /shown -->

`click_text` clicks the center of the matching text, runs the domain's
pending work and draws another frame. It fails if the text is missing or
ambiguous. `has_text` checks what the latest frame drew.

The same harness can mount the window returned by `app`. This checks the
declared initial route and the layout's navigation button:

<!-- shown: egui: testing an application -->
```rust
#[guinea::test]
fn the_about_button_moves_the_window(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut shell = Mounted::window(h, window);
    assert!(shell.has_text("0 by 1"));

    shell.click_text("About");

    assert!(shell.has_text("guinea"));
    assert!(!shell.has_text("0 by 1"));
}
```
<!-- /shown -->

Enable guinea's `eframe` and `harness` features to use
`guinea::eframe::harness::Mounted`.
:::
::::
