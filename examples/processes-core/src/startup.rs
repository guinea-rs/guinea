use std::time::{Duration, Instant};

use guinea::prelude::*;
use guinea_plugin_store::Store;

use crate::events::ProcessKilled;

#[derive(Clone)]
pub struct StartedAt(pub Instant);

const LAUNCHES: [&str; 2] = ["app", "launches"];

#[derive(Debug, Clone)]
pub struct Sweep;

#[derive(Default, Debug)]
pub struct Housekeeping {
    sweeps: u64,
}

actor! {
    Housekeeping {
        handlers { Sweep }
    }
}

#[handler]
fn sweep(this: &mut Housekeeping, _: Sweep) {
    this.sweeps += 1;
    tracing::debug!(sweeps = this.sweeps, "housekeeping sweep");
}

pub struct Startup;

impl AppFeature for Startup {
    type Exports = ();

    #[given]
    fn install(self, app: &mut FeatureBuilder, store: Store) -> anyhow::Result<()> {
        let started = Instant::now();
        app.provide(StartedAt(started));

        let launches = store.get::<u64>(LAUNCHES)?.unwrap_or_default() + 1;
        store.set(LAUNCHES, &launches)?;
        tracing::info!(launches, "started");

        let housekeeping = app.spawn(Housekeeping::default());
        app.every(Duration::from_secs(5), || Sweep, housekeeping)
            .named("housekeeping");

        app.subscribe::<ProcessKilled>(|event| {
            tracing::info!(process = %event.name, "process killed");
        });

        app.on_cleanup(move || {
            let uptime = started.elapsed();
            tracing::info!(?uptime, "shutdown ran");
            Ok(())
        });

        Ok(())
    }
}
