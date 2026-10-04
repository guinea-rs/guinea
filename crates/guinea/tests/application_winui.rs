#![cfg(all(windows, feature = "winui"))]

//! A route tree that hangs from its application, mounted under the harness:
//! with the application as it is, or with a test's stand-ins for what it
//! lists and nothing of what it installs besides.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;
use guinea::winui::{Page, PageCx, page};
use windows_reactor::{TextBlock, View};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link(pub &'static str);

impl Reducer for Link {
    type Update = &'static str;

    fn reduce(&mut self, to: &'static str) {
        self.0 = to;
    }
}

/// What the application lists: pages read `Link` from it.
pub struct Agents;

impl AppFeature for Agents {
    type Exports = (Link,);

    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.state::<Link>().seed(Link("real")).plain();
        Ok(())
    }
}

/// A test's stand-in for `Agents`, exporting what it does.
pub struct FakeAgents;

impl AppFeature for FakeAgents {
    type Exports = (Link,);

    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.state::<Link>().seed(Link("fake")).plain();
        Ok(())
    }
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

app! {
    pub App {
        installs { Agents }
    }
}

#[installs]
fn app(app: &mut FeatureBuilder) -> anyhow::Result<App> {
    app.plugin(Loud)?;
    Ok(App(app.feature(Agents)?))
}

#[derive(Default)]
pub struct Home;

#[page]
impl Page for Home {
    type Params = HomeParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (link, _) = cx.read::<Link>();
        TextBlock::new().text(format!("link {}", link.0)).into()
    }
}

guinea::routes! {
    Route {
        app(App) {
            page(Home) { }
        }
    }
}

/// Whether the tree mounted and drew `text`, or what stopped it.
fn shows(h: &Harness, text: &str) -> Result<bool, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut app = Mounted::routed(h, Route::Home {}).map_err(|error| format!("{error:#}"))?;
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
fn a_tree_mounts_under_the_application_as_it_is(h: &mut Harness) {
    LOUD_BUILT.set(false);

    let installed = h.install_application::<App>().map(|_| ());
    assert!(installed.is_ok(), "{installed:?}");

    assert_eq!(shows(h, "link real"), Ok(true));
    assert!(LOUD_BUILT.get());
}

#[guinea::test(iterations = 2)]
fn a_tree_mounts_under_a_test_standing_in_for_the_application(h: &mut Harness) {
    LOUD_BUILT.set(false);

    let installed = h
        .install_application_with(|app| Ok(App(app.feature_as(FakeAgents)?)))
        .map(|_| ());
    assert!(installed.is_ok(), "{installed:?}");

    assert_eq!(shows(h, "link fake"), Ok(true));
    assert!(!LOUD_BUILT.get(), "what the application installs besides is left out");
}
