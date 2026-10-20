use amethystate::store::builder::Backend;
use amethystate::{StoreBuilder, amethystate};
use guinea::prelude::*;

use crate::{Repository, Snapshot, configure};

#[amethystate(prefix = "todo")]
struct SavedBoard {
    #[amestate(default = Snapshot::default())]
    snapshot: Snapshot,
}

plugin! {
    pub DiskStorage {
        provides { Repository }
    }
}

#[builds]
fn disk_storage(app: &mut PluginBuilder, builder: StoreBuilder) -> anyhow::Result<DiskStorage> {
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
    Ok(DiskStorage(repository))
}

pub fn startup(app: &mut FeatureBuilder, backend: &str) -> anyhow::Result<()> {
    let builder = StoreBuilder::located(|at| at.app("guinea-todo", backend))?;
    let DiskStorage(repository) = app.plugin::<DiskStorage>(builder)?;
    configure(app, repository)
}
