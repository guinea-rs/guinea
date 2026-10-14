//! A plugin that stops the application before it starts: `run` returns `Ok`
//! without taking the terminal, and what was installed is torn down.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use guinea_app::app::{GuineaApp, Plugin, PluginBuilder, Stop};
use guinea_ratatui::Window;

struct Opened(Arc<AtomicBool>);

impl Plugin for Opened {
    const ID: &'static str = "test.opened";
    type Exports = ();

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.on_cleanup(move || {
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

    let ran = guinea_ratatui::run(app, |_| -> Window { panic!("nothing is opened after a stop") });

    assert!(ran.is_ok(), "{:#}", ran.unwrap_err());
    assert!(closed.load(Ordering::SeqCst), "the plugin before it was torn down");
}
