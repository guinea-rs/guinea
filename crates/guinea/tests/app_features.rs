//! An application's own feature is written as a window's is - `feature!` and
//! `#[installs]` - and the recipe installs it by type.

use guinea::app::Harness;
use guinea::prelude::*;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Language(pub &'static str);

#[reducer]
fn language(this: &mut Language, to: &'static str) {
    this.0 = to;
}

feature! {
    pub Localisation {
        exports { Language }
    }
}

#[installs]
fn localisation(app: &mut FeatureBuilder) -> anyhow::Result<Localisation> {
    Ok(Localisation(app.state::<Language>().seed(Language("en")).plain()))
}

feature! {
    pub Greeting {}
}

#[installs]
fn greeting(app: &mut FeatureBuilder) -> anyhow::Result<Greeting> {
    let localisation = app.install::<Localisation>()?;
    localisation.0.push("de");
    Ok(Greeting)
}

#[guinea::test(iterations = 1)]
fn a_page_reads_what_an_application_feature_the_recipe_installs_exports(h: &mut Harness) {
    let mut app = GuineaApp::new();
    app.install::<Localisation>();
    h.app(app).unwrap();

    let page = h.child();

    assert_eq!(page.context().read::<Language>().map(|it| it.0), Some("en"));
}

#[guinea::test(iterations = 1)]
fn an_application_feature_wires_another_through_its_handles(h: &mut Harness) {
    let mut app = GuineaApp::new();
    app.install::<Greeting>();
    h.app(app).unwrap();

    assert_eq!(h.child().context().read::<Language>().map(|it| it.0), Some("de"));
}

#[guinea::test(iterations = 1)]
fn an_application_feature_installed_twice_is_an_error_that_names_it(h: &mut Harness) {
    let mut app = GuineaApp::new();
    app.install::<Localisation>().install::<Greeting>();

    let outcome = h.app(app).map(|_| ()).map_err(|error| format!("{error:#}"));

    assert!(matches!(&outcome, Err(error) if error.contains("Localisation")), "{outcome:?}");
}
