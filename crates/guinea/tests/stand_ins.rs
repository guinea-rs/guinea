//! A stand-in installed with `plugin_as` or `feature_as` is, to the rest of
//! the application, the thing it stands in for.

use std::cell::RefCell;

use guinea::app::Harness;
use guinea::prelude::*;

thread_local! {
    static BUILT: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn built(what: &'static str) {
    BUILT.with(|built| built.borrow_mut().push(what));
}

fn so_far() -> Vec<&'static str> {
    BUILT.with(|built| built.borrow().clone())
}

pub struct Real;

impl Plugin for Real {
    const ID: &'static str = "test.real";
    type Exports = ();

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        built("real");
        Ok(())
    }
}

pub struct Fake;

impl Plugin for Fake {
    const ID: &'static str = "test.fake";
    type Exports = ();

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        built("fake");
        Ok(())
    }
}

pub struct Dependent;

impl Plugin for Dependent {
    const ID: &'static str = "test.dependent";
    type Exports = ();

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(Real)?;
        built("dependent");
        Ok(())
    }
}

pub struct Standing;

impl Application for Standing {
    fn install(app: &mut FeatureBuilder) -> anyhow::Result<Self> {
        app.plugin(Real)?;
        app.plugin(Dependent)?;
        Ok(Standing)
    }
}

pub struct RealWiring;

impl AppFeature for RealWiring {
    type Exports = ();

    fn install(self, _app: &mut FeatureBuilder) -> anyhow::Result<()> {
        built("real wiring");
        Ok(())
    }
}

pub struct FakeWiring;

impl AppFeature for FakeWiring {
    type Exports = ();

    fn install(self, _app: &mut FeatureBuilder) -> anyhow::Result<()> {
        built("fake wiring");
        Ok(())
    }
}

pub struct DependentWiring;

impl AppFeature for DependentWiring {
    type Exports = ();

    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.feature(RealWiring)?;
        built("dependent wiring");
        Ok(())
    }
}

#[guinea::test(iterations = 1)]
fn a_stand_in_stands_in_for_a_plugin_another_plugin_pulls(h: &mut Harness) {
    BUILT.with(|built| built.borrow_mut().clear());

    h.install_application_with::<Standing>(|app| {
        app.plugin_as::<Real, _>(Fake)?;
        app.plugin(Dependent)?;
        Ok(Standing)
    })
    .unwrap();

    assert_eq!(so_far(), ["fake", "dependent"], "the real plugin was built beside its stand-in");
}

#[guinea::test(iterations = 1)]
fn a_stand_in_stands_in_for_a_feature_another_feature_pulls(h: &mut Harness) {
    BUILT.with(|built| built.borrow_mut().clear());

    h.install_application_with::<Standing>(|app| {
        app.feature_as::<RealWiring, _>(FakeWiring)?;
        app.feature(DependentWiring)?;
        Ok(Standing)
    })
    .unwrap();

    assert_eq!(
        so_far(),
        ["fake wiring", "dependent wiring"],
        "the real feature was installed beside its stand-in"
    );
}

#[guinea::test(iterations = 1)]
fn the_application_has_what_a_stand_in_stands_in_for(h: &mut Harness) {
    h.install_application_with::<Standing>(|app| {
        app.feature_as::<RealWiring, _>(FakeWiring)?;
        app.plugin_as::<Real, _>(Fake)?;
        Ok(Standing)
    })
    .unwrap();

    let scope = h.application().scope;
    assert!(scope.has_feature::<RealWiring>(), "the application lacks the feature stood in for");
    assert!(scope.has_feature::<Real>(), "the application lacks the plugin stood in for");
}
