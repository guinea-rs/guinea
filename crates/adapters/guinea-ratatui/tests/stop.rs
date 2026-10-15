//! A plugin that stops the application before it starts: `run` returns `Ok`
//! without taking the terminal, and what was installed is torn down.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use guinea_app::app::{PluginBuilder, Stop};
use guinea_macros::{builds, plugin};
use guinea_ratatui::Window;

plugin! {
    Opened {}
}

#[builds]
fn opened(app: &mut PluginBuilder, closed: Arc<AtomicBool>) -> anyhow::Result<Opened> {
    app.on_cleanup(move || {
        closed.store(true, Ordering::SeqCst);
        Ok(())
    });
    Ok(Opened)
}

plugin! {
    SecondCopy {}
}

#[builds]
fn second_copy(_app: &mut PluginBuilder) -> anyhow::Result<SecondCopy> {
    Err(Stop.into())
}

#[test]
fn a_plugin_that_stops_the_application_ends_run_quietly() {
    let closed = Arc::new(AtomicBool::new(false));

    let opened = closed.clone();
    let ran = guinea_ratatui::run(move |app| -> anyhow::Result<Window> {
        app.plugin::<Opened>(opened)?;
        app.plugin::<SecondCopy>(())?;
        panic!("nothing is opened after a stop")
    });

    assert!(ran.is_ok(), "{:#}", ran.unwrap_err());
    assert!(closed.load(Ordering::SeqCst), "the plugin before it was torn down");
}
