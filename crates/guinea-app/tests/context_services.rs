use std::cell::Cell;
use std::rc::Rc;

use guinea_app::app::{PluginBuilder, roots::Root, start};
use guinea_app::feature::Install;
use guinea_macros::{builds, feature, installs, plugin};

#[derive(Clone)]
struct Store(Rc<Cell<u64>>);

plugin! {
    MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder, initial: u64) -> anyhow::Result<MemoryStore> {
    Ok(MemoryStore(Store(Rc::new(Cell::new(initial)))))
}

trait SettingsAccess {
    fn settings(&self) -> Option<Store>;
}

impl<Params: ?Sized> SettingsAccess for Install<'_, Params> {
    fn settings(&self) -> Option<Store> {
        self.service::<Store>()
    }
}

feature! {
    ReadSettings {}
}

#[installs]
fn read_settings(cx: &Install<u64>, injected: Store) -> anyhow::Result<ReadSettings> {
    let settings = cx.settings().expect("the application provided settings");
    assert!(Rc::ptr_eq(&settings.0, &injected.0));
    assert_eq!(settings.0.get(), *cx.params());
    settings.0.set(settings.0.get() + 1);
    Ok(ReadSettings)
}

#[test]
fn an_install_extension_reads_the_explicitly_selected_store() {
    let (host, chosen) = start("test", |app| {
        let MemoryStore(chosen) = app.plugin::<MemoryStore>(41)?;
        let MemoryStore(other) = app.plugin::<MemoryStore>(99)?;
        assert!(app.service::<Store>().is_none());
        app.provide(chosen.clone());
        assert_eq!(other.0.get(), 99);
        Ok(chosen)
    })
    .unwrap();
    let (_other_host, ()) = start("other", |app| {
        app.provide(Store(Rc::new(Cell::new(100))));
        Ok(())
    })
    .unwrap();
    let root = Root::under(&host.context());
    let _page = root
        .install(|cx| {
            cx.install::<ReadSettings>(&41)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(chosen.0.get(), 42);
}

#[test]
fn a_returned_but_unprovided_service_is_not_visible_to_a_context() {
    let (host, _unprovided) = start("test", |app| {
        let MemoryStore(store) = app.plugin::<MemoryStore>(7)?;
        Ok(store)
    })
    .unwrap();
    let root = Root::under(&host.context());
    let _page = root
        .install(|cx| {
            assert!(cx.service::<Store>().is_none());
            Ok(())
        })
        .unwrap();
}
