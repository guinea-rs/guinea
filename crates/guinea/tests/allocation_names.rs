#![cfg(all(feature = "allocations", feature = "harness"))]

use guinea::app::{Harness, Plugin, PluginBuilder};
use guinea::observability::trace::allocations::{Config, Profiler};

struct Storage;

impl Plugin for Storage {
    type Params = ();
    type Exports = ();

    fn build(_: &mut PluginBuilder, (): ()) -> anyhow::Result<Self> {
        Ok(Self)
    }
}

#[test]
fn application_window_and_plugin_have_allocation_names() {
    let profiler = Profiler::install(Config::default()).unwrap();
    let mut app = Harness::new(1);
    app.plugin::<Storage>(()).unwrap();
    let application = app.application_scope().allocation_group().unwrap().id();
    let window = app.segment().scope().allocation_group().unwrap().id();
    let scopes = profiler.snapshot();
    let names = |id| {
        scopes
            .iter()
            .find(|scope| scope.id == id)
            .unwrap()
            .declarations
            .iter()
            .map(|item| (item.kind, item.name))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(application),
        [
            ("role", "Application"),
            ("plugin", "allocation_names::Storage")
        ]
    );
    assert_eq!(names(window), [("role", "Window root")]);
}
