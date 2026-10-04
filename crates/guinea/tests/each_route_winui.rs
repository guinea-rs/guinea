#![cfg(all(windows, feature = "winui"))]

//! Every route of a tree, mounted and drawn once: what a read that reaches
//! nothing, or a fill nobody placed, would otherwise first show a user.

use std::rc::Rc;

use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, layout, page};
use windows_reactor::{StackPanel, TextBlock, View};

#[guinea::slot]
pub struct Toolbar;

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Missing(u32);

impl guinea::prelude::Reducer for Missing {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    type Params = ShellParams;

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((cx.slot::<Toolbar>(), cx.outlet()))
            .into()
    }
}

#[derive(Default)]
pub struct Good;

#[page]
impl Page for Good {
    type Params = GoodParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("good tools").into());
        TextBlock::new().text("good").into()
    }
}

#[derive(Default)]
pub struct Bad;

#[page]
impl Page for Bad {
    type Params = BadParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (missing, _) = cx.read::<Missing>();
        TextBlock::new().text(format!("bad {}", missing.0)).into()
    }
}

#[derive(Default)]
pub struct Lost;

#[page]
impl Page for Lost {
    type Params = LostParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("lost tools").into());
        TextBlock::new().text("lost").into()
    }
}

#[derive(Default)]
pub struct Numbered;

#[page]
impl Page for Numbered {
    type Params = NumberedParams;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("numbered").into()
    }
}

pub struct Handle;

#[derive(Default)]
pub struct Wired;

#[page]
impl Page for Wired {
    type Params = WiredParams;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("wired").into()
    }
}

guinea::routes! {
    Checked {
        layout(Shell) {
            page(Good) { }
            page(Bad) { }
            page(Numbered) { pid: u32 }
            page(Wired) { handle: ~Rc<Handle> }
        }
        page(Lost) { }
    }
}

#[guinea::test(iterations = 1)]
fn every_route_is_mounted_and_every_failure_is_told_at_once(h: &mut Harness) {
    let outcome = Mounted::each_route::<Checked>(h);

    let said = format!("{:#}", outcome.expect_err("two routes fail and one cannot be made"));
    assert!(said.contains("`Bad` reads `Missing`, but nothing reaches it"), "{said}");
    assert!(said.contains("`Lost` fills `Toolbar`, but no layout above it places that slot"), "{said}");
    assert!(said.contains("`Wired` was not mounted"), "{said}");
    assert!(!said.contains("Good") && !said.contains("Numbered"), "{said}");
}

mod clean {
    use super::*;

    #[derive(Default)]
    pub struct Frame;

    #[layout]
    impl Layout for Frame {
        type Params = FrameParams;

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            StackPanel::new()
                .children((cx.slot::<Toolbar>(), cx.outlet()))
                .into()
        }
    }

    #[derive(Default)]
    pub struct Fine;

    #[page]
    impl Page for Fine {
        type Params = FineParams;

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            cx.fill::<Toolbar>(TextBlock::new().text("fine tools").into());
            TextBlock::new().text("fine").into()
        }
    }

    #[derive(Default)]
    pub struct Counted;

    #[page]
    impl Page for Counted {
        type Params = CountedParams;

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("counted").into()
        }
    }

    guinea::routes! {
        Clean {
            layout(Frame) {
                page(Fine) { }
                page(Counted) { pid: u32 }
            }
        }
    }

    #[guinea::test(iterations = 1)]
    fn a_tree_whose_routes_all_draw_passes(h: &mut Harness) {
        Mounted::each_route::<Clean>(h).unwrap();
    }
}
