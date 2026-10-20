use guinea::prelude::*;

//@show composition: routes and shared metrics
#[derive(Clone, PartialEq, Debug, guinea::Routable)]
pub enum Place {
    Workspace(InWorkspace),
    About,
}

#[derive(Default, Clone, PartialEq, Debug, guinea::Routable)]
pub enum InWorkspace {
    #[default]
    Listing,
    Details {
        pid: u32,
    },
    Settings,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Level(pub u32);

#[reducer]
fn level(this: &mut Level, by: u32) {
    this.0 += by;
}

feature! {
    pub Metrics {
        exports { Level }
    }
}

#[installs]
fn metrics(app: &mut FeatureBuilder) -> anyhow::Result<Metrics> {
    let level = app.state::<Level>().plain();
    app.repeat(std::time::Duration::from_secs(1), move || level.push(1));
    Ok(Metrics(level))
}
//@show-end
