use std::sync::Arc;
use std::time::{Duration, Instant};

use guinea::prelude::*;
use guinea_plugin_store::Store;

use crate::events::ProcessKilled;

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
    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.provide(StartedAt(Instant::now()));
        let started: Arc<StartedAt> = app.require()?;

        let store = app.require::<Store>()?;
        let launches = store.get::<u64>(LAUNCHES)?.unwrap_or_default() + 1;
        store.set(LAUNCHES, &launches)?;
        tracing::info!(launches, "started");

        let addr = app.spawn(Housekeeping::default());
        app.every(Duration::from_secs(5), &addr, || Sweep)
            .named("housekeeping");

        app.subscribe_global::<ProcessKilled>(|event| {
            tracing::info!(process = %event.name, "process killed");
        });

        app.on_cleanup(move |_| {
            let uptime = started.0.elapsed();
            tracing::info!(?uptime, "shutdown ran");
            Ok(())
        });

        Ok(())
    }
}
