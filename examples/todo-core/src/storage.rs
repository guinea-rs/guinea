use amethystate::store::builder::Backend;
use amethystate::{StoreBuilder, amethystate};
use guinea::prelude::*;

use crate::{Repository, Snapshot, TodoApp};

#[amethystate(prefix = "todo")]
struct SavedBoard {
    #[amestate(default = Snapshot::default())]
    snapshot: Snapshot,
}

plugin! {
    pub StoragePlugin {
        provides { Repository }
    }
}

pub enum StorageConfig {
    Disk(StoreBuilder),
    Existing(Repository),
}

#[builds]
fn storage(app: &mut PluginBuilder, config: StorageConfig) -> anyhow::Result<StoragePlugin> {
    let repository = match config {
        StorageConfig::Disk(builder) => disk_repository(app, builder)?,
        StorageConfig::Existing(repository) => repository,
    };
    let final_save = repository.clone();
    app.on_cleanup(move || (final_save.save)(final_save.pending.borrow().clone()));
    Ok(StoragePlugin(repository))
}

fn disk_repository(app: &mut PluginBuilder, builder: StoreBuilder) -> anyhow::Result<Repository> {
    let store = builder.backend(Backend::Json).build()?;
    let closing = store.clone();
    app.on_cleanup(move || {
        closing.close()?;
        Ok(())
    });
    let settings = SavedBoard::try_new_with(&store)?;
    let snapshot = settings.snapshot();
    let repository = Repository::new(snapshot.get(), move |value| {
        snapshot.set(value);
        store.save_now()?;
        Ok(())
    });
    Ok(repository)
}

pub fn startup(app: &mut FeatureBuilder, backend: &str) -> anyhow::Result<()> {
    let builder = StoreBuilder::located(|at| at.app("guinea-todo", backend))?;
    app.plugin::<StoragePlugin>(StorageConfig::Disk(builder))?;
    app.install::<TodoApp>()?;
    Ok(())
}
