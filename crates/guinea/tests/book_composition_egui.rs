#![cfg(all(feature = "eframe", feature = "harness"))]

#[path = "support/composition.rs"]
mod common;

use common::*;
use guinea::app::Harness;
use guinea::eframe::harness::Mounted;
use guinea::eframe::{Layout, LayoutCx, Page, PageCx, Window, layout, page};
use guinea::prelude::*;

//@show egui: composition shell
#[derive(Default)]
pub struct Shell {
    last: InWorkspace,
}

#[layout]
impl Layout for Shell {
    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Place>();
        if cx.ui().button("About").clicked() {
            if let Some(Place::Workspace(level)) = nav.current() {
                self.last = level;
            }
            let _ = nav.to(Place::About);
        }
        if cx.ui().button("Workspace").clicked() {
            let _ = nav.to(Place::Workspace(self.last.clone()));
        }
        let charts = cx.mount::<Charts>(&());
        let content = cx.outlet(&nav, |place, cx| match place {
            Place::Workspace(_) => cx.mount::<Workspace>(&()).keep(),
            Place::About => cx.mount::<About>(&()),
        });
        charts.draw(cx.ui());
        content.draw(cx.ui());
    }
}

fn app(app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    app.install::<Metrics>()?;
    Ok(Window::of::<Shell>().at(Place::Workspace(InWorkspace::Listing)))
}
//@show-end

//@show egui: composition workspace
#[derive(Default)]
pub struct Workspace;

#[layout]
impl Layout for Workspace {
    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<InWorkspace>();
        if cx.ui().button("Listing").clicked() {
            let _ = nav.to(InWorkspace::Listing);
        }
        if cx.ui().button("Settings").clicked() {
            let _ = nav.to(InWorkspace::Settings);
        }
        if cx.ui().button("Details 7").clicked() {
            let _ = nav.to(InWorkspace::Details { pid: 7 });
        }
        if cx.ui().button("Details 8").clicked() {
            let _ = nav.to(InWorkspace::Details { pid: 8 });
        }
        if cx.ui().button("Details 13").clicked() {
            let _ = nav.to(InWorkspace::Details { pid: 13 });
        }
        let content = cx.outlet(&nav, |place, cx| match place {
            InWorkspace::Listing => cx.mount::<Listing>(&()).keep(),
            InWorkspace::Details { pid } => cx.mount::<Details>(pid),
            InWorkspace::Settings => cx.mount::<Settings>(&()),
        });
        content.draw(cx.ui());
    }
}

#[derive(Default)]
pub struct Charts;

#[page]
impl Page for Charts {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (level, _) = cx.read::<Level>();
        cx.ui().label(format!("charts {}", level.0));
    }
}
//@show-end

//@show egui: composition listing
#[derive(Default)]
pub struct Listing {
    visits: u32,
}

#[page]
impl Page for Listing {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        cx.ui().label(format!("listing visits {}", self.visits));
        if cx.ui().button("Select row").clicked() {
            self.visits += 1;
        }
    }
}
//@show-end

//@show egui: composition parameterized page
pub struct Details {
    pid: u32,
    visits: u32,
    help: bool,
}

#[page]
impl Page for Details {
    type Params = u32;

    fn install(cx: &Install<u32>) -> anyhow::Result<Self> {
        anyhow::ensure!(*cx.params() != 13, "process is unavailable");
        Ok(Self {
            pid: *cx.params(),
            visits: 0,
            help: false,
        })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let ui = cx.ui();
        ui.label(format!("process {} visits {}", self.pid, self.visits));
        if ui.button("Inspect").clicked() {
            self.visits += 1;
        }
        if ui.button("Help").clicked() {
            self.help = !self.help;
        }
        if self.help {
            ui.label("inspection help");
        }
    }
}
//@show-end

//@show egui: composition plain pages
#[derive(Default)]
pub struct About;

#[page]
impl Page for About {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        cx.ui().label("about the application");
    }
}

#[derive(Default)]
pub struct Settings;

#[page]
impl Page for Settings {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        cx.ui().label("workspace settings");
    }
}
//@show-end

//@show egui: composition preserves the right instances
#[guinea::test(iterations = 4)]
fn keep_and_parameters_choose_which_local_state_survives(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut ui = Mounted::window(h, window);
    ui.click_text("Select row");
    ui.click_text("Settings");
    ui.click_text("Listing");
    assert!(ui.has_text("listing visits 1"));

    ui.click_text("Details 7");
    ui.click_text("Inspect");
    ui.click_text("About");
    ui.click_text("Workspace");
    assert!(ui.has_text("process 7 visits 1"));
    ui.click_text("Details 8");
    assert!(ui.has_text("process 8 visits 0"));
    ui.click_text("Details 7");
    assert!(ui.has_text("process 7 visits 0"));
}
//@show-end

#[guinea::test(iterations = 4)]
fn the_chart_and_help_belong_to_their_own_pages(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut ui = Mounted::window(h, window);
    ui.click_text("Details 7");
    ui.click_text("Help");
    assert!(ui.has_text("inspection help"));
    h.advance(std::time::Duration::from_secs(1));
    ui.settle();
    assert!(ui.has_text("charts 1"));
    ui.click_text("Settings");
    assert!(!ui.has_text("inspection help"));
}

//@show egui: composition failed installation
#[guinea::test(iterations = 4)]
fn a_failed_details_install_keeps_the_previous_page_and_its_local_state(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut ui = Mounted::window(h, window);
    ui.click_text("Details 7");
    ui.click_text("Inspect");
    ui.click_text("Details 13");
    assert!(ui.has_text("process 7 visits 1"));
}
//@show-end
