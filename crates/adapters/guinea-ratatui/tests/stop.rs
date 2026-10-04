//! A plugin that stops the application before it starts: `run` returns `Ok`
//! without taking the terminal, and what was installed is torn down.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use guinea_app::app::{GuineaApp, Plugin, PluginBuilder, Stop};
use guinea_app::feature::FeatureInitContext;
use guinea_macros::routes;
use guinea_ratatui::{Flow, PageCx};

struct Home;

impl guinea_ratatui::Page for Home {
    type Params = HomeParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &HomeParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn render(_cx: &mut PageCx<'_, '_, Self>) {}
}

routes! {
    backend = guinea_ratatui::Tui,
    Route {
        page(Home) link("/")
    }
}

struct Opened(Arc<AtomicBool>);

impl Plugin for Opened {
    const ID: &'static str = "test.opened";
    type Exports = ();

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.on_cleanup(move |_| {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        });
        Ok(())
    }
}

struct SecondCopy;

impl Plugin for SecondCopy {
    const ID: &'static str = "test.second-copy";
    type Exports = ();

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        Err(Stop.into())
    }
}

#[test]
fn a_plugin_that_stops_the_application_ends_run_quietly() {
    let closed = Arc::new(AtomicBool::new(false));

    let app = GuineaApp::new()
        .plugin(Opened(closed.clone()))
        .plugin(SecondCopy);

    let ran = guinea_ratatui::run(
        app,
        |_: &_| -> Route { panic!("nothing is opened after a stop") },
        |_, _, _| Flow::Exit,
    );

    assert!(ran.is_ok(), "{:#}", ran.unwrap_err());
    assert!(closed.load(Ordering::SeqCst), "the plugin before it was torn down");
}
