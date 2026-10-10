//! The UI thread is the one the dispatcher was installed on. A binary of its
//! own: the dispatcher is the process's, and would reach the other tests.

use guinea_app::app::GuineaApp;
use guinea_core::actor::{UiDispatcher, UiTask, set_ui_dispatcher};

struct Nowhere;

impl UiDispatcher for Nowhere {
    fn init(&self) {}

    fn dispatch(&self, _: UiTask) {}
}

#[test]
fn an_application_installs_on_the_ui_thread_and_not_off_it() {
    set_ui_dispatcher(Nowhere);
    let on_it = GuineaApp::new().install().map(drop).map_err(|e| e.to_string());

    std::thread::spawn(|| set_ui_dispatcher(Nowhere)).join().unwrap();
    let off_it = GuineaApp::new().install().map(drop).map_err(|e| e.to_string());

    assert_eq!(
        (on_it, off_it),
        (
            Ok(()),
            Err("installed off the UI thread: the dispatcher runs work on another thread, \
                 where this application's actors would not be"
                .to_string())
        )
    );
}
