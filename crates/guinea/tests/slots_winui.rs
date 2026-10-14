#![cfg(all(windows, feature = "winui"))]

//! A layout places a slot, and the page under it fills it.
//!
//! The layout draws before its child, so a fill reaches the slot through a
//! placeholder of its own, a drain later - and goes with the segment that
//! made it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, Route, UpdateCx, layout, page};
use windows_reactor::{Button, StackPanel, TextBlock, View};

#[derive(Clone, PartialEq, Debug, Route)]
pub enum Place {
    Processes,
    Services,
    Toggling,
    Footing,
    Area(InArea),
}

#[derive(Clone, PartialEq, Debug, Route)]
pub enum InArea {
    Deep,
    Plain,
}

#[guinea::slot]
pub struct Toolbar;

#[guinea::slot]
pub struct Footer;

/// Mounted with the shell in its footer, with state of its own.
#[derive(Default)]
pub struct Charts {
    shown: u32,
}

pub enum More {
    Charts,
}

#[page]
impl Page for Charts {
    type Message = More;

    fn update(&mut self, message: More, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            More::Charts => self.shown += 1,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                TextBlock::new().text(format!("charts {}", self.shown)),
                Button::new()
                    .on_click(cx.on(|()| More::Charts))
                    .content(TextBlock::new().text("more charts")),
            ))
            .into()
    }
}

/// Covers the shell's footer with its own.
#[derive(Default)]
pub struct Footing;

#[page]
impl Page for Footing {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Footer>(TextBlock::new().text("page footer").into());
        TextBlock::new().text("footing").into()
    }
}

thread_local! {
    static SHELL_DRAWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        SHELL_DRAWN.set(SHELL_DRAWN.get() + 1);

        let nav = cx.navigate::<Place>();
        let toolbar = cx.slot::<Toolbar>();
        let page = cx.outlet(&nav, |place, cx| match place {
            Place::Processes => cx.mount::<Processes>(&()),
            Place::Services => cx.mount::<Services>(&()),
            Place::Toggling => cx.mount::<Toggling>(&()),
            Place::Footing => cx.mount::<Footing>(&()),
            Place::Area(_) => cx.mount::<Area>(&()),
        });
        let charts = cx.mount::<Charts>(&());
        let footer = cx.slot_or::<Footer>(charts);

        StackPanel::new().children((toolbar, page, footer)).into()
    }
}

#[derive(Default)]
pub struct Processes;

#[page]
impl Page for Processes {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("processes tools").into());
        TextBlock::new().text("processes").into()
    }
}

#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("services").into()
    }
}

#[derive(Default)]
pub struct Area;

#[layout]
impl Layout for Area {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("area tools").into());

        let nav = cx.navigate::<InArea>();
        cx.outlet(&nav, |place, cx| match place {
            InArea::Deep => cx.mount::<Deep>(&()),
            InArea::Plain => cx.mount::<Plain>(&()),
        })
    }
}

#[derive(Default)]
pub struct Deep;

#[page]
impl Page for Deep {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("deep tools").into());
        TextBlock::new().text("deep").into()
    }
}

#[derive(Default)]
pub struct Plain;

#[page]
impl Page for Plain {
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
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("nowhere").into());
        TextBlock::new().text("orphan").into()
    }
}

#[guinea::test(iterations = 4)]
fn a_part_shows_in_its_slot_where_nothing_below_fills_it(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
    app.settle();

    assert!(app.find_text("charts 0").is_some(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn a_fill_covers_the_part_and_leaving_uncovers_it_as_it_was(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
    app.settle();
    app.click_text("more charts").settle();
    app.settle();
    assert!(app.find_text("charts 1").is_some(), "{:#?}", app.tree());

    app.navigate(Place::Footing);
    app.settle();
    assert!(app.find_text("page footer").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("charts 1").is_none(), "{:#?}", app.tree());

    app.navigate(Place::Services);
    app.settle();
    assert!(
        app.find_text("charts 1").is_some(),
        "the part came back with the state it had:\n{:#?}",
        app.tree()
    );
}

#[guinea::test(iterations = 4)]
fn a_fill_redraws_the_slot_and_not_the_layout_that_placed_it(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Toggling).unwrap();
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
        let mut app = Mounted::<Orphan>::mount(h.segment(), ()).unwrap();
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
    let mut app = Mounted::routed::<Shell>(h, Place::Processes).unwrap();
    app.settle();

    assert!(app.find_text("processes tools").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("processes").is_some(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn leaving_the_page_takes_its_fill_with_it(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Processes).unwrap();
    app.settle();
    assert!(app.find_text("processes tools").is_some(), "{:#?}", app.tree());

    app.navigate(Place::Services);
    app.settle();

    assert!(app.find_text("services").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("processes tools").is_none(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn the_innermost_fill_wins(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Area(InArea::Deep)).unwrap();
    app.settle();

    assert!(app.find_text("deep tools").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("area tools").is_none(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn an_outer_fill_shows_where_the_inner_page_fills_nothing(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Area(InArea::Plain)).unwrap();
    app.settle();

    assert!(app.find_text("area tools").is_some(), "{:#?}", app.tree());
}

#[guinea::test(iterations = 4)]
fn a_fill_not_made_again_is_withdrawn(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Toggling).unwrap();
    app.settle();
    assert!(app.find_text("toggled tools").is_none(), "{:#?}", app.tree());

    app.click_text("flip").settle();
    app.settle();
    assert!(app.find_text("toggled tools").is_some(), "{:#?}", app.tree());

    app.click_text("flip").settle();
    app.settle();
    assert!(app.find_text("toggled tools").is_none(), "{:#?}", app.tree());
}

mod moved {
    use super::*;
    use windows_reactor::{Border, Grid};

    #[guinea::slot]
    pub struct Tools;

    #[derive(Default)]
    pub struct Mover {
        wrapped: bool,
    }

    pub enum Move {
        Flip,
    }

    #[layout]
    impl Layout for Mover {
        type Message = Move;

        fn update(&mut self, message: Move, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Move::Flip => self.wrapped = !self.wrapped,
            }
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let slot = cx.slot::<Tools>();
            let holder: View = match self.wrapped {
                true => Grid::new().children((slot,)).into(),
                false => Border::new().content(slot).into(),
            };
            let filler: View = cx.mount::<Filler>(&()).into();

            StackPanel::new()
                .children((
                    Button::new()
                        .on_click(cx.on(|()| Move::Flip))
                        .content(TextBlock::new().text("move")),
                    holder,
                    filler,
                ))
                .into()
        }
    }

    #[derive(Default)]
    pub struct Filler {
        n: u32,
    }

    pub enum Inc {
        Once,
    }

    #[page]
    impl Page for Filler {
        type Message = Inc;

        fn update(&mut self, message: Inc, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Inc::Once => self.n += 1,
            }
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            cx.fill::<Tools>(TextBlock::new().text(format!("fill {}", self.n)).into());
            StackPanel::new()
                .children((
                    TextBlock::new().text(format!("page {}", self.n)),
                    Button::new()
                        .on_click(cx.on(|()| Inc::Once))
                        .content(TextBlock::new().text("inc")),
                ))
                .into()
        }
    }

    #[guinea::test(iterations = 2)]
    fn a_slot_the_layout_moved_still_shows_what_fills_it(h: &mut Harness) {
        let mut app = Mounted::<Mover>::mount(h.segment(), ()).unwrap();
        app.settle();
        assert!(app.find_text("fill 0").is_some(), "{:#?}", app.tree());

        app.click_text("move").settle();
        app.settle();
        assert!(app.find_text("fill 0").is_some(), "{:#?}", app.tree());

        app.click_text("inc").settle();
        app.settle();
        assert!(app.find_text("page 1").is_some(), "{:#?}", app.tree());
        assert!(
            app.find_text("fill 1").is_some(),
            "the page filled the slot anew after the layout moved it:\n{:#?}",
            app.tree()
        );
    }
}
