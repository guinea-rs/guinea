//! What a feature, a plugin or an application feature needs it takes as
//! arguments, and the application provides: the signature is the whole list.

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
        cx: &FeatureInitContext,
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
    fn needy(_cx: &FeatureInitContext, _greeting: Greeting, _volume: Volume) -> anyhow::Result<Needy> {
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

    pub struct Greets;

    impl Plugin for Greets {
        const ID: &'static str = "test.greets";
        type Exports = ();

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            app.provide(Greeting("hello"));
            Ok(())
        }
    }

    pub struct Amplifies;

    impl Plugin for Amplifies {
        const ID: &'static str = "test.amplifies";
        type Exports = ();

        #[given]
        fn build(self, app: &mut PluginBuilder, greeting: Greeting) -> anyhow::Result<()> {
            app.provide(Volume(greeting.0.len() as u8));
            Ok(())
        }
    }

    pub struct Quiets;

    impl AppFeature for Quiets {
        type Exports = ();

        #[given]
        fn install(self, app: &mut FeatureBuilder, volume: Option<Volume>) -> anyhow::Result<()> {
            if volume.is_none() {
                app.provide(Volume(1));
            }
            Ok(())
        }
    }
}

#[guinea::test(iterations = 1)]
fn a_feature_takes_what_the_application_provided_as_its_arguments(h: &mut Harness) {
    h.provide(Greeting("hi"));
    h.provide(Volume(3));

    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi at 3");
}

#[guinea::test(iterations = 1)]
fn an_optional_argument_nobody_provided_is_none(h: &mut Harness) {
    h.provide(Greeting("hi"));

    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi at 0");
}

#[guinea::test(iterations = 1)]
fn a_feature_missing_what_it_takes_says_everything_it_misses_and_does_not_run(h: &mut Harness) {
    let refused = installed(h.install::<needy::Needy>(&())).unwrap_err();

    assert!(
        refused.contains("Needy") && refused.contains("Greeting") && refused.contains("Volume"),
        "{refused}"
    );
    assert!(!needy::RAN.get());
}

#[guinea::test(iterations = 1)]
fn what_the_segment_hands_in_is_read_through_the_install(h: &mut Harness) {
    h.provide(Greeting("hi"));

    assert_eq!(installed(h.install::<named::Named>(&"Ann".to_string())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi, Ann");
}

#[guinea::test(iterations = 1)]
fn a_plugin_takes_what_an_earlier_plugin_provided(h: &mut Harness) {
    h.plugin(plugins::Greets).unwrap();

    assert_eq!(installed(h.plugin(plugins::Amplifies)), Ok(()));
    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hello at 5");
}

#[guinea::test(iterations = 1)]
fn a_plugin_built_before_what_it_takes_is_refused_by_name(h: &mut Harness) {
    let refused = installed(h.plugin(plugins::Amplifies)).unwrap_err();

    assert!(refused.contains("Amplifies") && refused.contains("Greeting"), "{refused}");
}

#[guinea::test(iterations = 1)]
fn an_application_feature_takes_what_it_needs_as_arguments(h: &mut Harness) {
    h.provide(Greeting("hi"));

    assert_eq!(installed(h.feature(plugins::Quiets)), Ok(()));
    assert_eq!(installed(h.install::<greeter::Greeter>(&())), Ok(()));
    assert_eq!(h.state::<Said>().0, "hi at 1");
}

#[guinea::test(iterations = 1)]
fn a_service_provided_twice_is_a_mistake_that_names_the_first_provider(h: &mut Harness) {
    h.plugin(plugins::Greets).unwrap();

    let twice = catch_unwind(AssertUnwindSafe(|| {
        h.provide(Greeting("again"));
    }));
    let refused = twice.err().map(said).unwrap_or_default();

    assert!(refused.contains("Greeting") && refused.contains("test.greets"), "{refused}");
}

#[test]
fn whoever_holds_the_application_reads_what_was_provided() {
    let mut app = TestApp::new();
    app.install(plugins::Greets).unwrap();

    assert_eq!(app.service::<Greeting>(), Some(Greeting("hello")));
    assert_eq!(app.service::<Volume>(), None);
}
