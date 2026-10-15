use std::cell::RefCell;
use std::rc::Rc;

use guinea_core::actor::shape::name;
use guinea_macros::{builds, feature, installs, plugin};

use super::{FeatureBuilder, PluginBuilder, start};

thread_local! {
    static TRACE: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn trace(step: &'static str) {
    TRACE.with(|t| t.borrow_mut().push(step));
}

fn taken() -> Vec<&'static str> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

fn builder() -> FeatureBuilder {
    TRACE.with(|t| t.borrow_mut().clear());
    FeatureBuilder::new()
}

plugin! {
    /// Opens a store in a file of its own.
    Settings {
        provides { Store }
    }
}

#[builds]
fn settings(_app: &mut PluginBuilder, file: &'static str) -> anyhow::Result<Settings> {
    trace(file);
    Ok(Settings(Store(file)))
}

#[derive(Clone, Debug, PartialEq)]
struct Store(&'static str);

plugin! {
    /// Remembers the language in whichever store it is handed.
    Remembering {}
}

#[builds]
fn remembering(_app: &mut PluginBuilder, store: Store) -> anyhow::Result<Remembering> {
    trace(store.0);
    Ok(Remembering)
}

feature! {
    Startup {}
}

#[installs]
fn startup(app: &mut FeatureBuilder) -> anyhow::Result<Startup> {
    trace("startup");
    let Settings(store) = app.plugin::<Settings>("db")?;
    app.provide(store);
    Ok(Startup)
}

plugin! {
    Tidy {}
}

#[builds]
fn tidy(app: &mut PluginBuilder) -> anyhow::Result<Tidy> {
    app.on_cleanup(|| {
        trace("tidied");
        Ok(())
    });
    Ok(Tidy)
}

#[test]
fn an_application_goes_when_its_runtime_is_let_go_of() {
    taken();
    let (runtime, _) = start("test", |app| app.plugin::<Tidy>(())).unwrap();
    assert_eq!(taken(), [""; 0]);

    drop(runtime);

    assert_eq!(taken(), ["tidied"]);
}

#[test]
fn a_plugin_hands_back_what_it_built() {
    let mut app = builder();

    let store = app
        .plugin::<Settings>("settings")
        .map(|Settings(store)| store)
        .map_err(|error| format!("{error:#}"));

    assert_eq!(store, Ok(Store("settings")));
}

#[test]
fn a_plugin_installed_twice_is_two_copies_each_handed_to_whoever_the_application_says() {
    let mut app = builder();

    let built = (|| {
        let Settings(user) = app.plugin::<Settings>("settings")?;
        let Settings(internal) = app.plugin::<Settings>("state")?;
        app.plugin::<Remembering>(internal)?;
        anyhow::Ok(user)
    })()
    .map_err(|error| format!("{error:#}"));

    assert_eq!(built, Ok(Store("settings")));
    assert_eq!(taken(), ["settings", "state", "state"]);
}

#[test]
fn what_a_plugin_hands_back_is_not_provided_until_the_application_provides_it() {
    let mut app = builder();

    let store = app.plugin::<Settings>("settings").map(|Settings(store)| store);
    let before = app.scope.service::<Store>();
    if let Ok(store) = store {
        app.provide(store);
    }

    assert_eq!(before, None);
    assert_eq!(app.scope.service::<Store>(), Some(Store("settings")));
}

#[test]
fn every_copy_of_a_plugin_is_listed() {
    let mut app = builder();
    let _ = app.plugin::<Settings>("settings");
    let _ = app.plugin::<Settings>("state");

    assert_eq!(
        crate::observability::installed_plugins(app.scope),
        [name::<Settings>(), name::<Settings>()]
    );
}

#[test]
fn the_backend_that_started_the_application_is_named_for_devtools() {
    let (runtime, ()) = start("egui", |_| Ok(())).unwrap();

    let app = runtime.context().__scope();
    assert_eq!(crate::observability::backend(app), Some("egui"));
}

#[test]
fn a_feature_installed_twice_is_refused_before_it_runs_again() {
    let mut app = builder();
    app.install::<Startup>().unwrap();
    let again = app.install::<Startup>().map(|_| ());

    assert!(matches!(&again, Err(error) if error.to_string().contains("Startup")), "{again:?}");
    assert_eq!(taken(), vec!["startup", "db"]);
}

#[test]
fn a_feature_may_install_plugins_and_provide_what_they_hand_back() {
    let mut app = builder();
    app.install::<Startup>().unwrap();

    assert_eq!(app.scope.service::<Store>().map(|store| store.0), Some("db"));
}

#[test]
fn subscriptions_taken_during_install_are_dropped_on_shutdown() {
    use guinea_core::actor::event_bus::Event;

    #[derive(Clone)]
    struct Tick;
    impl Event for Tick {}

    let app = PluginBuilder::new();
    let bus = app.scope.bus();
    let seen = Rc::new(RefCell::new(0usize));

    {
        let seen = seen.clone();
        app.subscribe::<Tick>(move |_| *seen.borrow_mut() += 1);
    }

    assert_eq!(bus.count_subscribers::<Tick>(), 1);

    drop(app);

    assert_eq!(bus.count_subscribers::<Tick>(), 0);
}

#[derive(Clone)]
struct Greeting(&'static str);

plugin! {
    Greets {
        provides { Greeting }
    }
}

#[builds]
fn greets(_app: &mut PluginBuilder) -> anyhow::Result<Greets> {
    Ok(Greets(Greeting("hello")))
}

#[test]
fn a_feature_installs_without_a_router_and_reaches_the_services() {
    let (runtime, ()) = start("test", |app| {
        let Greets(greeting) = app.plugin::<Greets>(())?;
        app.provide(greeting);
        Ok(())
    })
    .expect("install");

    let host = crate::feature::FeatureHost::under(&runtime.context());
    let scope = host
        .install(|ctx| {
            assert_eq!(ctx.scope.service::<Greeting>().map(|greeting| greeting.0), Some("hello"));
            ctx.subscribe(|_: Ping| trace("pinged"));
            Ok(())
        })
        .expect("install the feature");

    host.scope().bus().publish(Ping);
    assert_eq!(taken(), vec!["pinged"]);

    drop(scope);
    host.scope().bus().publish(Ping);
    assert!(
        taken().is_empty(),
        "the subscription is owned by the scope and ends with it"
    );
}

#[derive(Clone)]
struct Ping;

impl guinea_core::actor::event_bus::Event for Ping {}

plugin! {
    Opened {}
}

#[builds]
fn opened(app: &mut PluginBuilder) -> anyhow::Result<Opened> {
    trace("opened");
    app.on_cleanup(|| {
        trace("closed");
        Ok(())
    });
    Ok(Opened)
}

plugin! {
    SecondCopy {}
}

#[builds]
fn second_copy(_app: &mut PluginBuilder) -> anyhow::Result<SecondCopy> {
    Err(super::Stop.into())
}

plugin! {
    Broken {}
}

#[builds]
fn broken(_app: &mut PluginBuilder) -> anyhow::Result<Broken> {
    anyhow::bail!("broken")
}

#[test]
fn a_plugin_that_stops_the_application_has_what_came_before_it_torn_down() {
    taken();

    let error = start("test", |app| {
        app.plugin::<Opened>(())?;
        app.plugin::<SecondCopy>(())?;
        app.install::<Startup>()?;
        Ok(())
    })
    .err()
    .expect("stopped");

    assert!(error.is::<super::Stop>(), "{error:#}");
    assert_eq!(taken(), vec!["opened", "closed"], "nothing after it was built");
}

#[test]
fn a_plugin_that_fails_has_what_came_before_it_torn_down() {
    taken();

    let error = start("test", |app| {
        app.plugin::<Opened>(())?;
        app.plugin::<Broken>(()).map(drop)
    })
    .err()
    .expect("failed");

    assert!(!error.is::<super::Stop>(), "{error:#}");
    assert_eq!(taken(), vec!["opened", "closed"]);
}

mod exports {
    use guinea_core::scope::Scope;
    use guinea_macros::{builds, feature, installs, plugin, reducer};

    use super::super::Harness;
    use super::{FeatureBuilder, PluginBuilder, builder, start};
    use crate::app::AppFeature;

    #[derive(Clone, Debug, Default)]
    struct Language(&'static str);

    #[reducer]
    fn reduce_language(this: &mut Language, update: &'static str) {
        this.0 = update;
    }

    feature! {
        Localisation {
            exports { Language }
        }
    }

    #[installs]
    fn localisation(app: &mut FeatureBuilder) -> anyhow::Result<Localisation> {
        Ok(Localisation(app.state::<Language>().seed(Language("en")).plain()))
    }

    fn localised(app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.install::<Localisation>()?;
        Ok(())
    }

    struct Forgetful;

    impl AppFeature for Forgetful {
        type Exports = (Language,);

        fn install(_app: &mut FeatureBuilder) -> anyhow::Result<Self> {
            Ok(Forgetful)
        }
    }

    plugin! {
        Translations {
            exports { Language }
        }
    }

    #[builds]
    fn translations(app: &mut PluginBuilder) -> anyhow::Result<Translations> {
        Ok(Translations(app.state::<Language>().seed(Language("de")).plain()))
    }

    fn language(scope: Scope) -> Option<&'static str> {
        scope
            .owner_of::<Language>()
            .map(|owner| owner.state::<Language>().borrow().0)
    }

    #[test]
    fn a_page_reads_what_an_application_feature_exports() {
        let mut harness = Harness::new(0);
        harness.app(localised).unwrap();

        let page = harness.child();

        assert_eq!(language(harness.segment().context().scope), Some("en"));
        assert_eq!(language(page.context().scope), Some("en"));
    }

    #[test]
    fn a_window_reads_what_the_installed_application_exports() {
        let (runtime, ()) = start("test", localised).expect("install");

        let window = crate::feature::FeatureHost::under(&runtime.context());

        assert_eq!(language(window.scope()), Some("en"));
    }

    #[test]
    fn a_window_sits_under_the_application_it_was_opened_from_not_the_latest_one() {
        let (english, ()) = start("test", localised).expect("install");
        let _german = start("test", |app| app.plugin::<Translations>(())).expect("install");

        let window = crate::feature::FeatureHost::under(&english.context());

        assert_eq!(language(window.scope()), Some("en"));
        assert!(window.application() == Some(&english.context()));
    }

    #[test]
    fn a_feature_reads_what_the_application_exports() {
        let mut harness = Harness::new(0);
        harness.app(localised).unwrap();

        let page = harness.child();

        assert_eq!(page.context().read::<Language>().map(|it| it.0), Some("en"));
    }

    #[test]
    fn reading_what_nothing_in_reach_exports_is_none() {
        let harness = Harness::new(0);

        let read = harness.segment().context().read::<Language>();

        assert!(read.is_none(), "got {read:?}");
    }

    #[test]
    fn a_page_reads_what_a_plugin_exports() {
        let mut harness = Harness::new(0);
        harness.plugin::<Translations>(()).unwrap();

        let page = harness.child();

        assert_eq!(language(page.context().scope), Some("de"));
    }

    #[test]
    fn exporting_what_nothing_claimed_is_an_error_that_names_it() {
        let mut app = builder();

        let outcome = app.install::<Forgetful>().map(|_| ());

        assert!(
            matches!(&outcome, Err(error) if format!("{error:#}").contains("Language")),
            "got {outcome:?}"
        );
    }

    #[derive(Clone, Debug, Default)]
    struct Draft(u32);

    #[reducer]
    fn reduce_draft(this: &mut Draft, update: u32) {
        this.0 = update;
    }

    struct Halfhearted;

    impl AppFeature for Halfhearted {
        type Exports = (Language,);

        fn install(app: &mut FeatureBuilder) -> anyhow::Result<Self> {
            app.state::<Draft>().plain();
            Ok(Halfhearted)
        }
    }

    #[test]
    fn a_feature_refused_for_what_it_did_not_claim_keeps_nothing_it_did() {
        let mut app = builder();

        assert!(app.install::<Halfhearted>().is_err());

        assert!(!app.scope.claims::<Draft>());
    }
}

mod owners {
    use guinea_core::actor::shape::name;
    use guinea_macros::{actor, builds, feature, handler, installs, plugin};

    use crate::observability::{app_actors, installed_plugins};
    use super::{FeatureBuilder, PluginBuilder, start};

    pub struct Sweep;

    #[derive(Debug, Default)]
    pub struct Sweeper;

    actor! {
        Sweeper {
            handlers { Sweep }
        }
    }

    #[handler]
    fn sweep(_this: &mut Sweeper, _: Sweep) {}

    #[derive(Debug, Default)]
    pub struct Loose;

    actor! {
        Loose {
            handlers { Sweep }
        }
    }

    #[handler]
    fn loose(_this: &mut Loose, _: Sweep) {}

    feature! {
        Housekeeping {}
    }

    #[installs]
    fn housekeeping(app: &mut FeatureBuilder) -> anyhow::Result<Housekeeping> {
        app.plugin::<Tools>(())?;
        app.spawn(Sweeper);
        Ok(Housekeeping)
    }

    fn housekeeping_app(app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.install::<Housekeeping>()?;
        Ok(())
    }

    plugin! {
        Tools {}
    }

    #[builds]
    fn tools(app: &mut PluginBuilder) -> anyhow::Result<Tools> {
        app.spawn(Loose);
        Ok(Tools)
    }

    #[test]
    fn an_application_actor_names_the_feature_that_spawned_it() {
        let (runtime, ()) = start("test", housekeeping_app).expect("install");
        let app = runtime.context().__scope();

        let owner = |name: &str| {
            app_actors(app)
                .into_iter()
                .find(|actor| actor.type_name.ends_with(name))
                .map(|actor| actor.owner.feature)
        };
        assert_eq!(owner("Sweeper"), Some(Some(name::<Housekeeping>())));
        assert_eq!(owner("Loose"), Some(None), "a plugin is not a feature");
    }

    #[test]
    fn what_an_application_was_built_with_is_read_from_its_scope() {
        let mut harness = crate::app::Harness::new(0);
        harness.app(housekeeping_app).unwrap();
        let app = harness.application_scope();

        assert_eq!(installed_plugins(app), [name::<Tools>()]);
        assert_eq!(app_actors(app).len(), 2);
    }

    #[test]
    fn two_applications_on_one_thread_are_read_apart() {
        let mut first = crate::app::Harness::new(0);
        first.app(housekeeping_app).unwrap();
        let second = crate::app::Harness::new(0);

        assert_eq!(installed_plugins(first.application_scope()), [name::<Tools>()]);
        assert_eq!(installed_plugins(second.application_scope()), [""; 0]);
    }
}

mod renders {
    use guinea_core::observability::Rendering;
    use guinea_core::trace::Point;

    use crate::app::Harness;

    #[test]
    fn a_harness_hears_every_render_however_quick() {
        let harness = Harness::new(0);

        let act = harness.record("draw", || drop(Rendering::of("Quick")));

        assert!(
            act.chain()
                .has(|point| matches!(point, Point::Render { segment: "Quick", .. })),
            "a test asks which segments a cause redrew; the threshold is devtools' noise \
             control, and a test's draw is usually under it: {:?}",
            act.chain()
        );
    }
}
