//! What a feature or an application feature needs it takes as arguments, and
//! the application provides: the signature is the whole list. A plugin is
//! handed what it needs instead, and hands back what it built.

use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::{Harness, TestApp};
use guinea::prelude::*;

/// A service: a cheap handle, handed out by clone.
#[derive(Clone, Debug, PartialEq)]
pub struct Greeting(pub &'static str);

/// A service a feature can do without.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume(pub u8);

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .unwrap_or_default()
}

fn installed(result: anyhow::Result<impl Sized>) -> Result<(), String> {
    result.map(|_| ()).map_err(|error| format!("{error:#}"))
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Said(pub String);

#[reducer]
fn said_it(this: &mut Said, words: String) {
    this.0 = words;
}

mod greeter {
    use super::*;

    feature! {
        pub Greeter {
            exports { Said }
        }
    }

    #[installs]
    fn greeter(
        cx: &Install,
        greeting: Greeting,
        volume: Option<Volume>,
    ) -> anyhow::Result<Greeter> {
        let volume = volume.map_or(0, |volume| volume.0);
        let said = cx.state::<Said>().seed(Said(format!("{} at {volume}", greeting.0))).plain();
        Ok(Greeter(said))
    }
}

mod needy {
    use super::*;

    thread_local! {
        pub static RAN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    feature! {
        pub Needy {}
    }

    #[installs]
    fn needy(_cx: &Install,_greeting: Greeting, _volume: Volume) -> anyhow::Result<Needy> {
        RAN.set(true);
        Ok(Needy)
    }
}

mod named {
    use super::*;

    feature! {
        pub Named {
            exports { Said }
        }
    }

    #[installs]
    fn named(cx: &Install<String>, greeting: Greeting) -> anyhow::Result<Named> {
        let said = cx.state::<Said>().seed(Said(format!("{}, {}", greeting.0, cx.params()))).plain();
        Ok(Named(said))
    }
}

mod plugins {
    use super::*;

    plugin! {
        pub Greets {
            provides { Greeting }
        }
    }

    #[builds]
    fn greets(_app: &mut PluginBuilder) -> anyhow::Result<Greets> {
        Ok(Greets(Greeting("hello")))
    }

    plugin! {
        /// Handed the greeting it amplifies.
        pub Amplifies {
            provides { Volume }
        }
    }

    #[builds]
    fn amplifies(_app: &mut PluginBuilder, greeting: Greeting) -> anyhow::Result<Amplifies> {
        Ok(Amplifies(Volume(greeting.0.len() as u8)))
    }

    feature! {
        pub Quiets { provides { Volume } }
    }

    #[installs]
    fn quiets(_app: &mut FeatureBuilder) -> anyhow::Result<Quiets> {
        Ok(Quiets(Volume(1)))
    }
}

#[guinea::test(iterations = 1)]
fn a_feature_takes_what_the_application_provided_as_its_arguments(h: &mut Harness) {
    h.plugin::<crate::dependency_fixture::Fixture<_>>(Greeting("hi")).unwrap();
    h.plugin::<crate::dependency_fixture::Fixture<_>>(Volume(3)).unwrap();

    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi at 3");
}

#[guinea::test(iterations = 1)]
fn an_optional_argument_with_an_unavailable_declared_provider_panics(h: &mut Harness) {
    h.plugin::<crate::dependency_fixture::Fixture<_>>(Greeting("hi")).unwrap();

    let refused = catch_unwind(AssertUnwindSafe(|| h.install::<greeter::Greeter>(&()))).err().map(said).unwrap();
    insta::assert_snapshot!(refused.replace('\\', "/"));
}

#[guinea::test(iterations = 1)]
fn a_feature_missing_what_it_takes_says_everything_it_misses_and_does_not_run(h: &mut Harness) {
    let refused = catch_unwind(AssertUnwindSafe(|| h.install::<needy::Needy>(&()))).err().map(said).unwrap();
    insta::assert_snapshot!(refused.replace('\\', "/"));
    assert!(!needy::RAN.get());
}

#[guinea::test(iterations = 1)]
fn what_the_segment_hands_in_is_read_through_the_install(h: &mut Harness) {
    h.plugin::<crate::dependency_fixture::Fixture<_>>(Greeting("hi")).unwrap();

    assert_eq!(installed(h.install::<named::Named>(&"Ann".to_string())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi, Ann");
}

#[guinea::test(iterations = 1)]
fn what_one_plugin_built_is_handed_to_another_and_provided_to_features(h: &mut Harness) {
    let built = h.app(|app| {
        let plugins::Greets(greeting) = app.plugin::<plugins::Greets>(())?;
        app.plugin::<plugins::Amplifies>(greeting.clone())?;
        Ok(())
    });

    assert_eq!(installed(built), Ok(()));
    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hello at 5");
}

#[guinea::test(iterations = 1)]
fn an_application_feature_takes_what_it_needs_as_arguments(h: &mut Harness) {
    h.plugin::<crate::dependency_fixture::Fixture<_>>(Greeting("hi")).unwrap();

    assert_eq!(installed(h.app(|app| app.install::<plugins::Quiets>())), Ok(()));
    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi at 1");
}

#[guinea::test(iterations = 1)]
fn a_service_provided_twice_is_a_mistake_that_names_the_first_provider(h: &mut Harness) {
    h.app(|app| app.install::<plugins::Quiets>()).unwrap();

    let twice = catch_unwind(AssertUnwindSafe(|| {
        h.plugin::<crate::dependency_fixture::Fixture<_>>(Volume(2)).unwrap();
    }));
    let refused = twice.err().map(said).unwrap_or_default();

    insta::assert_snapshot!(refused.replace('\\', "/"));
}

mod counted {
    use super::*;

    /// A service that stays on the UI thread.
    #[derive(Clone, Default)]
    pub struct Visits(pub std::rc::Rc<std::cell::Cell<u32>>);

    feature! {
        pub Counted {}
    }

    #[installs]
    fn counted(_cx: &Install,visits: Visits) -> anyhow::Result<Counted> {
        visits.0.set(visits.0.get() + 1);
        Ok(Counted)
    }
}

#[guinea::test(iterations = 1)]
fn a_service_that_stays_on_the_ui_thread_is_taken_like_any_other(h: &mut Harness) {
    let visits = counted::Visits::default();
    h.plugin::<crate::dependency_fixture::Fixture<_>>(visits.clone()).unwrap();

    assert_eq!(installed(h.child().install::<counted::Counted>(&())), Ok(()));
    assert_eq!(visits.0.get(), 1);
}

#[test]
fn a_plugin_publishes_its_declared_dependencies() {
    let mut app = TestApp::new();
    let plugins::Greets(greeting) = app.plugin::<plugins::Greets>(()).unwrap();
    assert_eq!(greeting, Greeting("hello"));
}

#[path = "support/dependencies.rs"]
mod dependency_fixture;
