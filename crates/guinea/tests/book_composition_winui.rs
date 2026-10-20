#![cfg(all(windows, feature = "winui"))]

#[path = "support/composition.rs"]
mod common;

use common::*;
use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, UpdateCx, Window, layout, page};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows_reactor::{Button, StackPanel, TextBlock, View};

//@show winui: composition button helper
fn button(text: &'static str, run: impl Fn() + 'static) -> Button {
    Button::new()
        .on_click(run)
        .content(TextBlock::new().text(text))
}
//@show-end

//@show winui: composition redraw probe
thread_local! {
    static SHELL_DRAWS: Cell<u32> = const { Cell::new(0) };
}
//@show-end

//@show winui: composition shell
#[guinea::slot]
pub struct Toolbar;

#[guinea::slot]
pub struct Overlay;

#[derive(Default)]
pub struct Shell {
    last: Rc<RefCell<InWorkspace>>,
}

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        //@hide
        SHELL_DRAWS.set(SHELL_DRAWS.get() + 1);
        //@unhide
        let nav = cx.navigate::<Place>();
        let about = nav.clone();
        let workspace = nav.clone();
        let remember = self.last.clone();
        let restore = self.last.clone();
        let toolbar = cx.slot::<Toolbar>();
        let overlay = cx.slot::<Overlay>();
        let charts = cx.mount::<Charts>(&());
        let content = cx.outlet(&nav, |place, cx| match place {
            Place::Workspace(_) => cx.mount::<Workspace>(&()).keep(),
            Place::About => cx.mount::<About>(&()),
        });
        StackPanel::new()
            .children((
                button("About", move || {
                    if let Some(Place::Workspace(level)) = about.current() {
                        *remember.borrow_mut() = level;
                    }
                    let _ = about.to(Place::About);
                }),
                button("Workspace", move || {
                    let _ = workspace.to(Place::Workspace(restore.borrow().clone()));
                }),
                charts,
                toolbar,
                content,
                overlay,
            ))
            .into()
    }
}

fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    app.install::<Metrics>()?;
    Ok(Window::of::<Shell>().at(Place::Workspace(InWorkspace::Listing)))
}
//@show-end

//@show winui: composition workspace
#[derive(Default)]
pub struct Workspace;

#[layout]
impl Layout for Workspace {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<InWorkspace>();
        let listing = nav.clone();
        let settings = nav.clone();
        let seven = nav.clone();
        let eight = nav.clone();
        let broken = nav.clone();
        let content = cx.outlet(&nav, |place, cx| match place {
            InWorkspace::Listing => cx.mount::<Listing>(&()).keep(),
            InWorkspace::Details { pid } => cx.mount::<Details>(pid),
            InWorkspace::Settings => cx.mount::<Settings>(&()),
        });
        StackPanel::new()
            .children((
                button("Listing", move || {
                    let _ = listing.to(InWorkspace::Listing);
                }),
                button("Settings", move || {
                    let _ = settings.to(InWorkspace::Settings);
                }),
                button("Details 7", move || {
                    let _ = seven.to(InWorkspace::Details { pid: 7 });
                }),
                button("Details 8", move || {
                    let _ = eight.to(InWorkspace::Details { pid: 8 });
                }),
                button("Details 13", move || {
                    let _ = broken.to(InWorkspace::Details { pid: 13 });
                }),
                content,
            ))
            .into()
    }
}

#[derive(Default)]
pub struct Charts;

#[page]
impl Page for Charts {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (level, _) = cx.read::<Level>();
        TextBlock::new().text(format!("charts {}", level.0)).into()
    }
}
//@show-end

//@show winui: composition listing
#[derive(Default)]
pub struct Listing {
    visits: u32,
}

pub enum Edit {
    Visit,
    Help,
}

#[page]
impl Page for Listing {
    type Message = Edit;
    fn update(&mut self, _: Edit, _: &mut UpdateCx<'_, Self>) {
        self.visits += 1;
    }
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                TextBlock::new().text(format!("listing visits {}", self.visits)),
                Button::new()
                    .on_click(cx.on(|()| Edit::Visit))
                    .content(TextBlock::new().text("Select row")),
            ))
            .into()
    }
}
//@show-end

//@show winui: composition parameterized page and fill
pub struct Details {
    pid: u32,
    visits: u32,
    help: bool,
}

#[page]
impl Page for Details {
    type Params = u32;
    type Message = Edit;

    fn install(cx: &Install<u32>) -> anyhow::Result<Self> {
        anyhow::ensure!(*cx.params() != 13, "process is unavailable");
        Ok(Self {
            pid: *cx.params(),
            visits: 0,
            help: false,
        })
    }

    fn update(&mut self, message: Edit, _: &mut UpdateCx<'_, Self>) {
        match message {
            Edit::Visit => self.visits += 1,
            Edit::Help => self.help = !self.help,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        cx.fill::<Toolbar>(TextBlock::new().text(format!("tools {}", self.pid)).into());
        if self.help {
            cx.fill::<Overlay>(TextBlock::new().text("inspection help").into());
        }
        StackPanel::new()
            .children((
                TextBlock::new().text(format!("process {} visits {}", self.pid, self.visits)),
                Button::new()
                    .on_click(cx.on(|()| Edit::Visit))
                    .content(TextBlock::new().text("Inspect")),
                Button::new()
                    .on_click(cx.on(|()| Edit::Help))
                    .content(TextBlock::new().text("Help")),
            ))
            .into()
    }
}
//@show-end

//@show winui: composition plain pages
#[derive(Default)]
pub struct About;

#[page]
impl Page for About {
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("about the application").into()
    }
}

#[derive(Default)]
pub struct Settings;

#[page]
impl Page for Settings {
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("workspace settings").into()
    }
}
//@show-end

//@show winui: composition preserves the right instances
#[guinea::test(iterations = 4)]
fn keep_and_parameters_choose_which_local_state_survives(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut ui = Mounted::<Place>::from_window(h, window).unwrap();
    ui.settle();
    ui.click_text("Select row").settle();
    ui.click_text("Settings").settle();
    ui.click_text("Listing").settle();
    assert!(ui.find_text("listing visits 1").is_some());

    ui.click_text("Details 7").settle();
    ui.click_text("Inspect").settle();
    ui.click_text("About").settle();
    ui.click_text("Workspace").settle();
    assert!(ui.find_text("process 7 visits 1").is_some());
    ui.click_text("Details 8").settle();
    assert!(ui.find_text("process 8 visits 0").is_some());
    ui.click_text("Details 7").settle();
    assert!(ui.find_text("process 7 visits 0").is_some());
}
//@show-end

//@show winui: composition keeps redraws local
#[guinea::test(iterations = 4)]
fn metrics_and_slot_updates_do_not_rebuild_the_shell(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut ui = Mounted::<Place>::from_window(h, window).unwrap();
    ui.settle();
    ui.click_text("Details 7").settle();
    let before = SHELL_DRAWS.get();
    ui.click_text("Help").settle();
    assert!(ui.find_text("inspection help").is_some());
    assert_eq!(SHELL_DRAWS.get(), before);
    h.advance(std::time::Duration::from_secs(1));
    ui.settle();
    assert!(ui.find_text("charts 1").is_some());
    assert_eq!(SHELL_DRAWS.get(), before);
    ui.click_text("About").settle();
    assert!(ui.find_text("inspection help").is_none());
    assert!(ui.find_text("tools 7").is_none());
    ui.click_text("Workspace").settle();
    assert!(ui.find_text("inspection help").is_some());
    ui.click_text("Settings").settle();
    assert!(ui.find_text("inspection help").is_none());
    assert!(ui.find_text("tools 7").is_none());
}
//@show-end

//@show winui: composition failed installation
#[guinea::test(iterations = 4)]
fn a_failed_details_install_keeps_the_previous_page_and_its_local_state(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut ui = Mounted::<Place>::from_window(h, window).unwrap();
    ui.settle();
    ui.click_text("Details 7").settle();
    ui.click_text("Inspect").settle();
    ui.click_text("Details 13").settle();
    assert!(ui.find_text("process 7 visits 1").is_some());
    assert_eq!(
        ui.route(),
        Place::Workspace(InWorkspace::Details { pid: 7 })
    );
}
//@show-end
