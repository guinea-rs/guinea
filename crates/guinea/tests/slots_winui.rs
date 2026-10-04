#![cfg(all(windows, feature = "winui"))]

//! A layout places a slot, and the page under it fills it.
//!
//! The layout draws before its child, so a fill reaches the slot through a
//! placeholder of its own, a drain later - and goes with the segment that
//! made it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, UpdateCx, layout, page};
use windows_reactor::{Button, StackPanel, TextBlock, View};

#[guinea::slot]
pub struct Toolbar;

thread_local! {
    static SHELL_DRAWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    type Params = ShellParams;

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        SHELL_DRAWN.set(SHELL_DRAWN.get() + 1);
        StackPanel::new()
            .children((cx.slot::<Toolbar>(), cx.outlet()))
            .into()
    }
}

#[derive(Default)]
pub struct Processes;

#[page]
impl Page for Processes {
    type Params = ProcessesParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("processes tools").into());
        TextBlock::new().text("processes").into()
    }
}

#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    type Params = ServicesParams;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("services").into()
    }
}

#[derive(Default)]
pub struct Area;

#[layout]
impl Layout for Area {
    type Params = AreaParams;

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("area tools").into());
        cx.outlet()
    }
}

#[derive(Default)]
pub struct Deep;

#[page]
impl Page for Deep {
    type Params = DeepParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("deep tools").into());
        TextBlock::new().text("deep").into()
    }
}

#[derive(Default)]
pub struct Plain;

#[page]
impl Page for Plain {
    type Params = PlainParams;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("plain").into()
    }
}

#[derive(Default)]
pub struct Toggling {
    filling: bool,
}

pub enum Toggle {
    Flip,
}

#[page]
impl Page for Toggling {
    type Params = TogglingParams;
    type Message = Toggle;

    fn update(&mut self, message: Toggle, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Toggle::Flip => self.filling = !self.filling,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        if self.filling {
            cx.fill::<Toolbar>(TextBlock::new().text("toggled tools").into());
        }

        Button::new()
            .on_click(cx.on(|()| Toggle::Flip))
            .content(TextBlock::new().text("flip"))
            .into()
    }
}

/// Fills a slot with no layout above it.
#[derive(Default)]
pub struct Orphan;

#[page]
impl Page for Orphan {
    type Params = OrphanParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("nowhere").into());
        TextBlock::new().text("orphan").into()
    }
}

guinea::routes! {
    SlotRoute {
        layout(Shell) {
            page(Processes) { }
            page(Services) { }
            page(Toggling) { }
            layout(Area) {
                page(Deep) { }
                page(Plain) { }
            }
        }
        page(Orphan) { }
    }
}

#[guinea::test(iterations = 4)]
fn a_fill_redraws_the_slot_and_not_the_layout_that_placed_it(h: &mut Harness) {
    let mut app = Mounted::routed(h, SlotRoute::Toggling {}).unwrap();
    app.settle();
    let before = SHELL_DRAWN.get();

    app.click_text("flip").settle();
    app.settle();

    assert!(app.find_text("toggled tools").is_some(), "{:#?}", app.tree());
    assert_eq!(SHELL_DRAWN.get(), before, "the shell drew again for its child's fill");
}

#[guinea::test(iterations = 1)]
fn filling_a_slot_nobody_placed_panics_naming_the_chain(h: &mut Harness) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut app = Mounted::routed(h, SlotRoute::Orphan {}).unwrap();
        app.settle();
    }));

    let message = match &outcome {
        Err(panic) => panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string())),
        Ok(()) => None,
    };
    assert!(
        message
            .as_deref()
            .is_some_and(|it| it.contains("`Orphan` fills `Toolbar`, but no layout above it places that slot")),
        "got {message:?}"
    );
}

#[guinea::test(iterations = 4)]
fn a_page_fills_the_slot_its_layout_placed(h: &mut Harness) {
    let mut app = Mounted::routed(h, SlotRoute::Processes {}).unwrap();
    app.settle();

    assert!(app.find_text("processes tools").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("processes").is_some(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn leaving_the_page_takes_its_fill_with_it(h: &mut Harness) {
    let mut app = Mounted::routed(h, SlotRoute::Processes {}).unwrap();
    app.settle();
    assert!(app.find_text("processes tools").is_some(), "{:#?}", app.tree());

    app.navigate(SlotRoute::Services {});
    app.settle();

    assert!(app.find_text("services").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("processes tools").is_none(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn the_innermost_fill_wins(h: &mut Harness) {
    let mut app = Mounted::routed(h, SlotRoute::Deep {}).unwrap();
    app.settle();

    assert!(app.find_text("deep tools").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("area tools").is_none(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn an_outer_fill_shows_where_the_inner_page_fills_nothing(h: &mut Harness) {
    let mut app = Mounted::routed(h, SlotRoute::Plain {}).unwrap();
    app.settle();

    assert!(app.find_text("area tools").is_some(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn a_fill_not_made_again_is_withdrawn(h: &mut Harness) {
    let mut app = Mounted::routed(h, SlotRoute::Toggling {}).unwrap();
    app.settle();
    assert!(app.find_text("toggled tools").is_none(), "{:#?}", app.tree());

    app.click_text("flip").settle();
    app.settle();
    assert!(app.find_text("toggled tools").is_some(), "{:#?}", app.tree());

    app.click_text("flip").settle();
    app.settle();
    assert!(app.find_text("toggled tools").is_none(), "{:#?}", app.tree());
}
