---
title: Pages and layouts
sidebar:
  order: 2
---

A feature holds what the application knows; a page shows it. A page is drawn
the way its toolkit draws, so this is where the backends differ - the tabs
below show the same counter on each. The rest of guinea is the same under all
of them: what a page installs, what it reads, and when it goes.

## A page

A page is a struct, and the struct is the page's own state - what only this
page has an opinion about, like the step it adds by. What a feature owns -
the count - it reads.

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

WinUI keeps a tree of controls and redraws what changed, so a page is written
the Elm way:

- **`view`** describes the page. `cx.read::<Count>()` hands back the state and
  a `Dispatch` to act on the feature that owns it, and subscribes the page:
  when `Count` changes, the page draws again.
- **`update`** changes the page's own state. `cx.on(|()| Msg::Bigger)` turns a
  widget's event into one of the page's messages, and the message arrives
  here.
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

egui draws every frame from scratch, so a page has no messages and no
`update`: **`render`** takes `&mut self`, and the frame that sees the click is
the frame that answers it. `cx.read::<Count>()` hands back the state and a
`Dispatch`; there is nothing to subscribe to, because the next frame reads it
again.
:::
::::

**`install`** runs once, when the page is mounted. What it installs lives in
the page's scope and goes when the page does - see [Scopes](../scopes/). It
returns the state the page starts with.

So there are two ways out of a click, and the choice says whose the state is.
A click that changes what the page shows of itself changes the page's own
state. A click that asks the domain for something goes to the feature through
`dispatch.emit`, and comes back as a new state to read.

`#[page]` writes what is left out: a page with no `install` starts from
`Default`, and a page with nothing to install needs no `Params`.

## A layout

A layout draws what is around its pages and mounts one of them. Where the
window is, is a route - an enum - and `cx.outlet` mounts the page for the
current one.

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

`cx.outlet` hands back the page's drawing instead of drawing it, so the layout
decides where in its `ui` the page goes.
:::
::::

`cx.navigate::<Place>()` is the navigator over `Place` nearest above, the one
the window opened. Moving it mounts the page for the new place and removes the
scope of the one that was there. A layout's `install` works the way a page's
does, and what it installs, the pages under it share.

## An application

An application is a function that installs what the application needs and
returns the window to open. `run` calls it, opens the window, and runs until
it closes.

::::backends
:::winui
<!-- shown: winui: an application -->
```rust
use guinea::winui::{Window, run};

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}

fn main() -> anyhow::Result<()> {
    run(app)
}
```
<!-- /shown -->
:::
:::egui
<!-- shown: egui: an application -->
```rust
use guinea::eframe::{Window, run};

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}

fn main() -> anyhow::Result<()> {
    run(app)
}
```
<!-- /shown -->
:::
::::

`Window::of::<Shell>()` is the layout the window roots at, and `.at(..)` where
its navigator starts. What the function installs with `app.install` lives in
the application's scope, above every window.

## Testing

A page mounts without a window, under the same harness as a feature, and its
drawing is read back as a tree.

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

`#[derive(guinea::Mark)]` and `.mark(..)` name the controls a test reaches
for, so a test does not depend on their text or their place in the tree;
`click_text` finds one by its text when that is simpler. `settle` runs
everything the click set off - the action, the actor's turn, the push into the
reducer, the page drawing again - in an order the seed decides.

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
:::
:::egui
egui has no harness for pages yet. The features behind a page are tested
without it, the way [Scopes](../scopes/) shows.
:::
::::
