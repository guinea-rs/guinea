use std::cell::RefCell;
use std::rc::Rc;

use guinea_core::actor::UiThreadToken;

use crate::lifecycle_tracker::AppLifecycle;

use super::{AppFeature, FeatureBuilder, Plugin, PluginBuilder};

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
    FeatureBuilder::new(
        UiThreadToken::dangerously_create_token_unchecked(),
        AppLifecycle::new(),
    )
}

struct Settings;
impl Plugin for Settings {
    const ID: &'static str = "test.settings";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        trace("settings");
        app.provide(Store("db"));
        Ok(())
    }
}

struct Store(&'static str);

struct Updater;
impl Plugin for Updater {
    const ID: &'static str = "test.updater";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(Settings)?;
        let store = app.require::<Store>()?;
        trace(store.0);
        Ok(())
    }
}

struct Left;
impl Plugin for Left {
    const ID: &'static str = "test.left";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(Settings)?;
        trace("left");
        Ok(())
    }
}

struct Colliding;
impl Plugin for Colliding {
    const ID: &'static str = "test.settings";
    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        Ok(())
    }
}

struct CycleA;
struct CycleB;

impl Plugin for CycleA {
    const ID: &'static str = "test.cycle.a";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(CycleB)?;
        Ok(())
    }
}

impl Plugin for CycleB {
    const ID: &'static str = "test.cycle.b";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(CycleA)?;
        Ok(())
    }
}

struct Orphan;
impl Plugin for Orphan {
    const ID: &'static str = "test.orphan";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.require::<Store>()?;
        Ok(())
    }
}

struct Startup;
impl AppFeature for Startup {
    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        trace("startup");
        app.plugin(Settings)?;
        Ok(())
    }
}

#[test]
fn a_plugin_pulled_twice_is_built_once() {
    let mut app = builder();
    app.plugin(Updater).unwrap();
    app.plugin(Left).unwrap();

    assert_eq!(taken(), vec!["settings", "db", "left"]);
}

#[test]
fn a_diamond_builds_the_shared_dependency_once() {
    let mut app = builder();
    app.plugin(Left).unwrap();
    app.plugin(Updater).unwrap();

    assert_eq!(taken(), vec!["settings", "left", "db"]);
}

#[test]
fn a_dependency_cycle_names_the_path() {
    let mut app = builder();
    let err = format!("{:#}", app.plugin(CycleA).map(|_| ()).unwrap_err());

    assert!(err.contains("cycle"), "got {err}");
    assert!(err.contains("test.cycle.a"), "got {err}");
    assert!(err.contains("test.cycle.b"), "got {err}");
}

#[test]
fn two_plugins_sharing_an_id_is_an_error() {
    let mut app = builder();
    app.plugin(Settings).unwrap();

    let err = format!("{:#}", app.plugin(Colliding).map(|_| ()).unwrap_err());
    assert!(err.contains("ID collision"), "got {err}");
}

#[test]
fn a_missing_service_names_who_wanted_it() {
    let mut app = builder();
    let err = format!("{:#}", app.plugin(Orphan).map(|_| ()).unwrap_err());

    assert!(err.contains("test.orphan"), "got {err}");
    assert!(err.contains("Store"), "got {err}");
}

#[test]
fn a_feature_installed_twice_runs_once() {
    let mut app = builder();
    app.feature(Startup).unwrap();
    app.feature(Startup).unwrap();

    assert_eq!(taken(), vec!["startup", "settings"]);
}

#[test]
fn a_feature_may_pull_plugins_and_read_what_they_provide() {
    let mut app = builder();
    app.feature(Startup).unwrap();

    assert_eq!(app.require::<Store>().unwrap().0, "db");
}

#[test]
fn subscriptions_taken_during_install_are_dropped_on_shutdown() {
    use guinea_core::actor::event_bus::{Event, GlobalEventBus};

    #[derive(Clone)]
    struct Tick;
    impl Event for Tick {}

    let token = UiThreadToken::dangerously_create_token_unchecked();
    let lifecycle = AppLifecycle::new();
    let seen = Rc::new(RefCell::new(0usize));

    {
        let app = PluginBuilder::new(token.clone(), lifecycle.clone());
        let seen = seen.clone();
        app.subscribe_global::<Tick>(move |_| *seen.borrow_mut() += 1);
    }

    assert_eq!(GlobalEventBus::count_subscribers::<Tick>(), 1);

    lifecycle.shutdown();

    assert_eq!(GlobalEventBus::count_subscribers::<Tick>(), 0);
}

struct Greeting(&'static str);

struct GreetingPlugin;

impl Plugin for GreetingPlugin {
    const ID: &'static str = "test.greeting";

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.provide(Greeting("hello"));
        Ok(())
    }
}

#[test]
fn a_feature_installs_without_a_router_and_reaches_the_services() {
    let token = UiThreadToken::dangerously_create_token_unchecked();

    let runtime = super::GuineaApp::new()
        .plugin(GreetingPlugin)
        .install(token.clone())
        .expect("install");
    crate::app::install_runtime(runtime);

    // No chain, no route, no backend: an application with a single window and
    // nothing to navigate between still gets a scope and its services.
    let host = crate::feature::FeatureHost::new(token);
    let scope = host
        .install(|ctx| {
            assert_eq!(ctx.require::<Greeting>()?.0, "hello");
            ctx.subscribe(|_: Ping| trace("pinged"));
            Ok(())
        })
        .expect("install the feature");

    host.event_bus().publish(Ping);
    assert_eq!(taken(), vec!["pinged"]);

    drop(scope);
    host.event_bus().publish(Ping);
    assert!(
        taken().is_empty(),
        "the subscription is owned by the scope and ends with it"
    );
}

#[derive(Clone)]
struct Ping;

impl guinea_core::actor::event_bus::Event for Ping {}

struct NeedsMeta;

impl Plugin for NeedsMeta {
    const ID: &'static str = "test.needs-meta";

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        let meta = app.require::<super::AppMeta>()?;
        trace(if meta.identifier == "dev.uniproc.test" {
            "read the identifier"
        } else {
            "read something else"
        });
        Ok(())
    }
}

#[test]
fn a_plugin_reads_the_application_identity_instead_of_being_told_it() {
    let token = UiThreadToken::dangerously_create_token_unchecked();

    super::GuineaApp::new()
        .meta(super::AppMeta::new(
            "Test",
            "dev.uniproc.test",
            "1.2.3",
            "uniproc",
        ))
        .plugin(NeedsMeta)
        .install(token)
        .expect("install");

    assert_eq!(taken(), vec!["read the identifier"]);
}

#[test]
fn meta_declared_after_a_plugin_is_still_there_for_it() {
    let token = UiThreadToken::dangerously_create_token_unchecked();

    // Order in the builder is registration order, not installation order -
    // both are replayed before any plugin is built.
    super::GuineaApp::new()
        .plugin(NeedsMeta)
        .meta(super::AppMeta::new(
            "Test",
            "dev.uniproc.test",
            "1.2.3",
            "uniproc",
        ))
        .install(token)
        .expect("install");

    assert_eq!(taken(), vec!["read the identifier"]);
}

struct Opened;

impl Plugin for Opened {
    const ID: &'static str = "test.opened";

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        trace("opened");
        app.on_cleanup(|_| {
            trace("closed");
            Ok(())
        });
        Ok(())
    }
}

struct SecondCopy;

impl Plugin for SecondCopy {
    const ID: &'static str = "test.second-copy";

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        Err(super::Stop.into())
    }
}

struct Broken;

impl Plugin for Broken {
    const ID: &'static str = "test.broken";

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        anyhow::bail!("broken")
    }
}

#[test]
fn a_plugin_that_stops_the_application_has_what_came_before_it_torn_down() {
    taken();

    let error = super::GuineaApp::new()
        .plugin(Opened)
        .plugin(SecondCopy)
        .feature(Startup)
        .install(UiThreadToken::dangerously_create_token_unchecked())
        .err()
        .expect("stopped");

    assert!(error.is::<super::Stop>(), "{error:#}");
    assert_eq!(taken(), vec!["opened", "closed"], "nothing after it was built");
}

#[test]
fn a_plugin_that_fails_has_what_came_before_it_torn_down() {
    taken();

    let error = super::GuineaApp::new()
        .plugin(Opened)
        .plugin(Broken)
        .install(UiThreadToken::dangerously_create_token_unchecked())
        .err()
        .expect("failed");

    assert!(!error.is::<super::Stop>(), "{error:#}");
    assert_eq!(taken(), vec!["opened", "closed"]);
}

mod exports {
    use guinea_core::actor::UiThreadToken;
    use guinea_core::scope::{Reducer, Scope};

    use super::super::Harness;
    use super::{AppFeature, FeatureBuilder, builder};

    #[derive(Clone, Debug, Default)]
    struct Language(&'static str);

    impl Reducer for Language {
        type Update = &'static str;

        fn reduce(&mut self, update: &'static str) {
            self.0 = update;
        }
    }

    struct Localisation;

    impl AppFeature for Localisation {
        fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
            app.state::<Language>().seed(Language("en")).plain();
            app.export::<Language>()?;
            Ok(())
        }
    }

    fn language(scope: Scope) -> Option<&'static str> {
        scope
            .owner_of::<Language>()
            .map(|owner| owner.state::<Language>().borrow().0)
    }

    #[test]
    fn a_page_reads_what_an_application_feature_exports() {
        let mut harness = Harness::new(0);
        harness.feature(Localisation).unwrap();

        let page = harness.child();

        assert_eq!(language(harness.segment().context().scope), Some("en"));
        assert_eq!(language(page.context().scope), Some("en"));
    }

    #[test]
    fn a_window_reads_what_the_installed_application_exports() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let runtime = super::super::GuineaApp::new()
            .feature(Localisation)
            .install(token.clone())
            .expect("install");
        crate::app::install_runtime(runtime);

        let window = crate::feature::FeatureHost::new(token);

        assert_eq!(language(window.scope()), Some("en"));
    }

    #[test]
    fn exporting_what_nothing_claimed_is_an_error_that_names_it() {
        let app = builder();

        let outcome = app.export::<Language>().map(|_| ());

        assert!(
            matches!(&outcome, Err(error) if format!("{error:#}").contains("Language")),
            "got {outcome:?}"
        );
    }
}

mod owners {
    use guinea_macros::{actor, handler};

    use super::super::actors::app_actors;
    use super::{AppFeature, FeatureBuilder, Plugin, PluginBuilder, builder};

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

    struct Housekeeping;

    impl AppFeature for Housekeeping {
        fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
            app.plugin(Tools)?;
            app.spawn(Sweeper);
            Ok(())
        }
    }

    struct Tools;

    impl Plugin for Tools {
        const ID: &'static str = "test.tools";

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            app.spawn(Loose);
            Ok(())
        }
    }

    #[test]
    fn an_application_actor_names_the_feature_that_spawned_it() {
        let mut app = builder();
        app.feature(Housekeeping).unwrap();

        let owner = |name: &str| {
            app_actors()
                .into_iter()
                .find(|actor| actor.type_name.ends_with(name))
                .map(|actor| actor.owner.feature)
        };
        assert_eq!(owner("Sweeper"), Some(Some(std::any::type_name::<Housekeeping>())));
        assert_eq!(owner("Loose"), Some(None), "a plugin is not a feature");
    }
}
