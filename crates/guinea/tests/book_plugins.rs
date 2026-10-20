use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use guinea::app::{Harness, TestApp};
use guinea::prelude::*;

//@show plugins: a plugin and its service
#[derive(Clone, Debug, Default)]
pub struct Store(Rc<RefCell<BTreeMap<String, String>>>);

impl Store {
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.borrow().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: &str) {
        self.0.borrow_mut().insert(key.to_owned(), value.to_owned());
    }
}

plugin! {
    pub MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder, theme: &'static str) -> anyhow::Result<MemoryStore> {
    let store = Store::default();
    store.set("theme", theme);
    Ok(MemoryStore(store))
}
//@show-end

//@show plugins: application wiring
fn app(app: &mut FeatureBuilder) -> anyhow::Result<Store> {
    let MemoryStore(user) = app.plugin::<MemoryStore>("dark")?;
    let MemoryStore(internal) = app.plugin::<MemoryStore>("light")?;
    app.provide(user);
    Ok(internal)
}
//@show-end

//@show plugins: a feature takes a service
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Theme(pub String);

#[reducer]
fn theme(this: &mut Theme, value: String) {
    this.0 = value;
}

feature! {
    pub Appearance {
        exports { Theme }
    }
}

#[installs]
fn appearance(cx: &Install, store: Store) -> anyhow::Result<Appearance> {
    let theme = cx
        .state::<Theme>()
        .seed(Theme(store.get("theme").unwrap_or_default()))
        .plain();
    Ok(Appearance(theme))
}
//@show-end

//@show plugins: testing application wiring
#[guinea::test]
fn the_application_chooses_which_store_its_pages_receive(h: &mut Harness) {
    let internal = h.app(app).unwrap();
    h.install::<Appearance>(&()).unwrap();

    assert_eq!(h.state::<Theme>().0, "dark");
    assert_eq!(internal.get("theme").as_deref(), Some("light"));

    internal.set("theme", "blue");
    let page = h.child();
    page.install::<Appearance>(&()).unwrap();
    assert_eq!(page.state::<Theme>().0, "dark");
}
//@show-end

//@show plugins: returned is not provided
#[test]
fn returning_a_service_does_not_publish_it() {
    let mut app = TestApp::new();
    let MemoryStore(store) = app.plugin::<MemoryStore>("dark").unwrap();

    assert!(app.service::<Store>().is_none());
    app.provide(store);
    assert_eq!(
        app.service::<Store>().unwrap().get("theme").as_deref(),
        Some("dark")
    );
}
//@show-end

//@show plugins: registering cleanup
plugin! {
    Cleanup {}
}

#[builds]
fn cleanup(app: &mut PluginBuilder, cleaned: Rc<Cell<bool>>) -> anyhow::Result<Cleanup> {
    app.on_cleanup(move || {
        cleaned.set(true);
        Ok(())
    });
    Ok(Cleanup)
}
//@show-end

//@show plugins: cleanup belongs to the application
#[test]
fn a_plugins_cleanup_runs_when_the_application_ends() {
    let cleaned = Rc::new(Cell::new(false));
    let mut app = TestApp::new();
    app.plugin::<Cleanup>(cleaned.clone()).unwrap();

    assert!(!cleaned.get());
    drop(app);
    assert!(cleaned.get());
}
//@show-end

//@show plugins: a partially built indexer
pub struct IndexingParams {
    pub directory: PathBuf,
    pub polls: Rc<Cell<u32>>,
}

plugin! {
    Indexing {}
}

#[builds]
fn indexing(app: &mut PluginBuilder, params: IndexingParams) -> anyhow::Result<Indexing> {
    let lock = params.directory.join("index.lock");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)?;
    app.on_cleanup(move || {
        std::fs::remove_file(lock)?;
        Ok(())
    });

    let polls = params.polls;
    app.repeat(Duration::from_secs(1), move || polls.set(polls.get() + 1));
    std::fs::write(params.directory.join("startup.log"), b"starting indexer\n")?;

    let _settings = std::fs::read_to_string(params.directory.join("index.toml"))?;
    Ok(Indexing)
}
//@show-end

//@show plugins: catching an error keeps the scope
#[guinea::test]
fn catching_an_indexer_error_leaves_its_timer_running(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let polls = Rc::new(Cell::new(0));
    let mut app = TestApp::new();
    let result = app.plugin::<Indexing>(IndexingParams {
        directory: directory.path().to_owned(),
        polls: polls.clone(),
    });

    assert!(result.is_err());
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 3);
    assert!(directory.path().join("index.lock").exists());

    drop(app);
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 3);
    assert!(!directory.path().join("index.lock").exists());
    assert!(directory.path().join("startup.log").exists());
}
//@show-end

//@show plugins: propagating an error drops the host
#[guinea::test]
fn an_indexer_error_propagated_out_of_startup_releases_the_scope(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let polls = Rc::new(Cell::new(0));

    let result = (|| -> anyhow::Result<TestApp> {
        let mut app = TestApp::new();
        app.plugin::<Indexing>(IndexingParams {
            directory: directory.path().to_owned(),
            polls: polls.clone(),
        })?;
        Ok(app)
    })();

    assert!(result.is_err());
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 0);
    assert!(!directory.path().join("index.lock").exists());
    assert!(directory.path().join("startup.log").exists());
}
//@show-end
