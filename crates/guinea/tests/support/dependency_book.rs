use std::cell::RefCell;
use std::rc::Rc;

use guinea::prelude::*;

pub struct StorageConfig(pub &'static str);

#[derive(Clone)]
pub struct Store(Rc<RefCell<String>>);

impl Store {
    pub fn rename(&self, title: &str) {
        *self.0.borrow_mut() = title.to_owned();
    }
}

#[derive(Clone)]
pub struct Session {
    pub user: &'static str,
    store: Store,
}

impl Session {
    fn new(store: Store) -> Self {
        Self {
            user: "Alice",
            store,
        }
    }
}

#[derive(Clone)]
pub struct Document(Store);

impl Document {
    fn open(store: Store, session: Session) -> anyhow::Result<Self> {
        anyhow::ensure!(Rc::ptr_eq(&store.0, &session.store.0), "different stores");
        Ok(Self(store))
    }

    pub fn title(&self) -> String {
        self.0.0.borrow().clone()
    }
}

fn open_store(config: StorageConfig) -> anyhow::Result<Store> {
    anyhow::ensure!(!config.0.is_empty(), "empty document title");
    Ok(Store(Rc::new(RefCell::new(config.0.to_owned()))))
}

//@show dependencies: plugin outputs
plugin! {
    pub Storage {
        provides { Store }
    }
}

#[builds]
fn storage(_app: &mut PluginBuilder, config: StorageConfig) -> anyhow::Result<Storage> {
    Ok(Storage(open_store(config)?))
}
//@show-end

//@show dependencies: application inputs and outputs
feature! {
    pub Application {
        provides { Session }
    }
}

#[installs]
fn application(_app: &mut FeatureBuilder, store: Store) -> anyhow::Result<Application> {
    Ok(Application(Session::new(store)))
}
//@show-end

//@show dependencies: feature inputs and outputs
feature! {
    pub Editor {
        provides { Document }
    }
}

#[installs]
fn editor(_cx: &Install, store: Store, session: Session) -> anyhow::Result<Editor> {
    Ok(Editor(Document::open(store, session)?))
}
//@show-end

//@show dependencies: application wiring
pub fn app(app: &mut FeatureBuilder) -> anyhow::Result<Store> {
    let Storage(store) = app.plugin::<Storage>(StorageConfig("Notes"))?;
    app.install::<Application>()?;
    Ok(store)
}
//@show-end
