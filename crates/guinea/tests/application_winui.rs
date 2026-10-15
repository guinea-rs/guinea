#![cfg(all(windows, feature = "winui"))]

//! A page that reads from its application, mounted under the harness: with
//! the application's recipe as it is, or with a test's own recipe that has a
//! fake where the application has the real thing.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;
use guinea::winui::{Page, PageCx, page};
use windows_reactor::{TextBlock, View};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link(pub &'static str);

#[reducer]
fn link(this: &mut Link, to: &'static str) {
    this.0 = to;
}

feature! {
    pub Agents {
        exports { Link }
    }
}

#[installs]
fn agents(app: &mut FeatureBuilder) -> anyhow::Result<Agents> {
    Ok(Agents(app.state::<Link>().seed(Link("real")).plain()))
}

feature! {
    pub FakeAgents {
        exports { Link }
    }
}

#[installs]
fn fake_agents(app: &mut FeatureBuilder) -> anyhow::Result<FakeAgents> {
    Ok(FakeAgents(app.state::<Link>().seed(Link("fake")).plain()))
}

thread_local! {
    static LOUD_BUILT: Cell<bool> = const { Cell::new(false) };
}

/// What the application installs and pages do not read: a single-instance
/// lock, devtools.
pub struct Loud;

impl Plugin for Loud {
    const ID: &'static str = "test.loud";
    type Exports = ();

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        LOUD_BUILT.set(true);
        Ok(())
    }
}

fn recipe() -> GuineaApp {
    let mut app = GuineaApp::new();
    app.plugin(Loud).install::<Agents>();
    app
}

#[derive(Default)]
pub struct Home;

#[page]
impl Page for Home {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (link, _) = cx.read::<Link>();
        TextBlock::new().text(format!("link {}", link.0)).into()
    }
}

/// Whether the page mounted and drew `text`, or what stopped it.
fn shows(h: &Harness, text: &str) -> Result<bool, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut app = Mounted::<Home>::mount(h.segment(), ()).map_err(|error| format!("{error:#}"))?;
        app.settle();
        Ok(app.find_text(text).is_some())
    }))
    .unwrap_or_else(|panic| {
        Err(panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string()))
            .unwrap_or_default())
    })
}

#[guinea::test(iterations = 2)]
fn a_page_mounts_under_the_application_as_it_is(h: &mut Harness) {
    LOUD_BUILT.set(false);

    let installed = h.app(recipe()).map(|_| ());
    assert!(installed.is_ok(), "{installed:?}");

    assert_eq!(shows(h, "link real"), Ok(true));
    assert!(LOUD_BUILT.get());
}

#[guinea::test(iterations = 2)]
fn a_page_mounts_under_a_test_s_own_recipe(h: &mut Harness) {
    LOUD_BUILT.set(false);

    let mut app = GuineaApp::new();
    app.install::<FakeAgents>();
    let installed = h.app(app).map(|_| ());
    assert!(installed.is_ok(), "{installed:?}");

    assert_eq!(shows(h, "link fake"), Ok(true));
    assert!(!LOUD_BUILT.get(), "what the test's recipe leaves out is not built");
}
