#![cfg(all(windows, feature = "winui"))]

//! Every place a route names, mounted and drawn once: what a read that
//! reaches nothing, or a fill nobody placed, would otherwise first show a
//! user.

use std::rc::Rc;

use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, Routable, layout, page};
use windows_reactor::{StackPanel, TextBlock, View};

#[guinea::slot]
pub struct Toolbar;

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Missing(u32);

#[guinea::reducer]
fn missing(this: &mut Missing, to: u32) {
    this.0 = to;
}

#[derive(PartialEq, Debug)]
pub struct Handle;

#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Checked {
    Shell(InShell),
    Lost,
}

#[derive(Clone, PartialEq, Debug, Routable)]
pub enum InShell {
    Good,
    Bad,
    Numbered { pid: u32 },
    Wired { handle: Rc<Handle> },
}

#[derive(Default)]
pub struct App;

#[layout]
impl Layout for App {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Checked>();
        cx.outlet(&nav, |place, cx| match place {
            Checked::Shell(_) => cx.mount::<Shell>(&()),
            Checked::Lost => cx.mount::<Lost>(&()),
        })
    }
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<InShell>();
        let toolbar = cx.slot::<Toolbar>();
        let page = cx.outlet(&nav, |place, cx| match place {
            InShell::Good => cx.mount::<Good>(&()),
            InShell::Bad => cx.mount::<Bad>(&()),
            InShell::Numbered { pid } => cx.mount::<Numbered>(pid),
            InShell::Wired { .. } => cx.mount::<Wired>(&()),
        });

        StackPanel::new().children((toolbar, page)).into()
    }
}

#[derive(Default)]
pub struct Good;

#[page]
impl Page for Good {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("good tools").into());
        TextBlock::new().text("good").into()
    }
}

#[derive(Default)]
pub struct Bad;

#[page]
impl Page for Bad {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (missing, _) = cx.read::<Missing>();
        TextBlock::new().text(format!("bad {}", missing.0)).into()
    }
}

#[derive(Default)]
pub struct Lost;

#[page]
impl Page for Lost {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text("lost tools").into());
        TextBlock::new().text("lost").into()
    }
}

#[derive(Default)]
pub struct Numbered;

#[page]
impl Page for Numbered {
    type Params = u32;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("numbered").into()
    }
}

#[derive(Default)]
pub struct Wired;

#[page]
impl Page for Wired {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("wired").into()
    }
}

#[guinea::test(iterations = 1)]
fn every_place_is_mounted_and_every_failure_is_told_at_once(h: &mut Harness) {
    let outcome = Mounted::each_route::<App, Checked>(h);

    let said = format!("{:#}", outcome.expect_err("two places fail and one cannot be made"));
    assert!(said.contains("`Bad` reads `Missing`, but nothing reaches it"), "{said}");
    assert!(said.contains("`Lost` fills `Toolbar`, but no layout above it places that slot"), "{said}");
    assert!(said.contains("`Wired` was not mounted"), "{said}");
    assert!(!said.contains("Good") && !said.contains("Numbered"), "{said}");
}

mod clean {
    use super::*;

    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum Clean {
        Fine,
        Counted { pid: u32 },
    }

    #[derive(Default)]
    pub struct Frame;

    #[layout]
    impl Layout for Frame {
        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<Clean>();
            let toolbar = cx.slot::<Toolbar>();
            let page = cx.outlet(&nav, |place, cx| match place {
                Clean::Fine => cx.mount::<Fine>(&()),
                Clean::Counted { pid } => cx.mount::<Counted>(pid),
            });

            StackPanel::new().children((toolbar, page)).into()
        }
    }

    #[derive(Default)]
    pub struct Fine;

    #[page]
    impl Page for Fine {
        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            cx.fill::<Toolbar>(TextBlock::new().text("fine tools").into());
            TextBlock::new().text("fine").into()
        }
    }

    #[derive(Default)]
    pub struct Counted;

    #[page]
    impl Page for Counted {
        type Params = u32;

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("counted").into()
        }
    }

    #[guinea::test(iterations = 1)]
    fn a_tree_whose_places_all_draw_passes(h: &mut Harness) {
        Mounted::each_route::<Frame, Clean>(h).unwrap();
    }
}
