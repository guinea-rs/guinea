use std::cell::RefCell;
use std::rc::Rc;

use guinea_macros::{feature, given, installs};

use super::{FeatureBuilder, GuineaApp, Plugin, PluginBuilder};

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

struct Settings;
impl Plugin for Settings {
    type Exports = ();

    const ID: &'static str = "test.settings";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        trace("settings");
        app.provide(Store("db"));
        Ok(())
    }
}

#[derive(Clone)]
struct Store(&'static str);

struct Updater;
impl Plugin for Updater {
    type Exports = ();

    const ID: &'static str = "test.updater";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(Settings)?;
        trace("updater");
        Ok(())
    }
}

struct Left;
impl Plugin for Left {
    type Exports = ();

    const ID: &'static str = "test.left";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(Settings)?;
        trace("left");
        Ok(())
    }
}

struct Colliding;
impl Plugin for Colliding {
    type Exports = ();

    const ID: &'static str = "test.settings";
    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        Ok(())
    }
}

struct CycleA;
struct CycleB;

impl Plugin for CycleA {
    type Exports = ();

    const ID: &'static str = "test.cycle.a";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(CycleB)?;
        Ok(())
    }
}

impl Plugin for CycleB {
    type Exports = ();

    const ID: &'static str = "test.cycle.b";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.plugin(CycleA)?;
        Ok(())
    }
}

struct Orphan;
impl Plugin for Orphan {
    type Exports = ();

    const ID: &'static str = "test.orphan";

    #[given]
    fn build(self, _app: &mut PluginBuilder, _store: Store) -> anyhow::Result<()> {
        Ok(())
    }
}

feature! {
    Startup {}
}

#[installs]
fn startup(app: &mut FeatureBuilder) -> anyhow::Result<Startup> {
    trace("startup");
    app.plugin(Settings)?;
    Ok(Startup)
}

struct Tidy;
impl Plugin for Tidy {
    type Exports = ();

    const ID: &'static str = "test.tidy";
    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.on_cleanup(|| {
            trace("tidied");
            Ok(())
        });
        Ok(())
    }
}

#[test]
fn an_application_goes_when_its_runtime_is_let_go_of() {
    taken();
    let mut app = GuineaApp::new();
    app.plugin(Tidy);
    let runtime = app.start().unwrap();
    assert_eq!(taken(), [""; 0]);

    drop(runtime);

    assert_eq!(taken(), ["tidied"]);
}

#[test]
fn a_plugin_pulled_twice_is_built_once() {
    let mut app = builder();
    app.plugin(Updater).unwrap();
    app.plugin(Left).unwrap();

    assert_eq!(taken(), vec!["settings", "updater", "left"]);
}

#[test]
fn a_diamond_builds_the_shared_dependency_once() {
    let mut app = builder();
    app.plugin(Left).unwrap();
    app.plugin(Updater).unwrap();

    assert_eq!(taken(), vec!["settings", "left", "updater"]);
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
fn a_feature_installed_twice_is_refused_before_it_runs_again() {
    let mut app = builder();
    app.install::<Startup>().unwrap();
    let again = app.install::<Startup>().map(|_| ());

    assert!(matches!(&again, Err(error) if error.to_string().contains("Startup")), "{again:?}");
    assert_eq!(taken(), vec!["startup", "settings"]);
}

#[test]
fn a_feature_may_pull_plugins_and_read_what_they_provide() {
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

struct GreetingPlugin;

impl Plugin for GreetingPlugin {
    type Exports = ();

    const ID: &'static str = "test.greeting";

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.provide(Greeting("hello"));
        Ok(())
    }
}

#[test]
fn a_feature_installs_without_a_router_and_reaches_the_services() {
    let mut app = GuineaApp::new();
    app.plugin(GreetingPlugin);
    let runtime = app.start().expect("install");

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

struct NeedsMeta;

impl Plugin for NeedsMeta {
    type Exports = ();

    const ID: &'static str = "test.needs-meta";

    #[given]
    fn build(self, _app: &mut PluginBuilder, meta: super::AppMeta) -> anyhow::Result<()> {
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
    let mut app = GuineaApp::new();
    app.meta(super::AppMeta::new("Test", "dev.uniproc.test", "1.2.3", "uniproc"))
        .plugin(NeedsMeta);
    app.start().expect("install");

    assert_eq!(taken(), vec!["read the identifier"]);
}

#[test]
fn meta_declared_after_a_plugin_is_still_there_for_it() {
    let mut app = GuineaApp::new();
    app.plugin(NeedsMeta)
        .meta(super::AppMeta::new("Test", "dev.uniproc.test", "1.2.3", "uniproc"));
    app.start().expect("install");

    assert_eq!(taken(), vec!["read the identifier"]);
}

struct Opened;

impl Plugin for Opened {
    type Exports = ();

    const ID: &'static str = "test.opened";

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        trace("opened");
        app.on_cleanup(|| {
            trace("closed");
            Ok(())
        });
        Ok(())
    }
}

struct SecondCopy;

impl Plugin for SecondCopy {
    type Exports = ();

    const ID: &'static str = "test.second-copy";

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        Err(super::Stop.into())
    }
}

struct Broken;

impl Plugin for Broken {
    type Exports = ();

    const ID: &'static str = "test.broken";

    fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<()> {
        anyhow::bail!("broken")
    }
}

#[test]
fn a_plugin_that_stops_the_application_has_what_came_before_it_torn_down() {
    taken();

    let mut app = GuineaApp::new();
    app.plugin(Opened).plugin(SecondCopy).install::<Startup>();
    let error = app.start().err().expect("stopped");

    assert!(error.is::<super::Stop>(), "{error:#}");
    assert_eq!(taken(), vec!["opened", "closed"], "nothing after it was built");
}

#[test]
fn a_plugin_that_fails_has_what_came_before_it_torn_down() {
    taken();

    let mut app = GuineaApp::new();
    app.plugin(Opened).plugin(Broken);
    let error = app.start().err().expect("failed");

    assert!(!error.is::<super::Stop>(), "{error:#}");
    assert_eq!(taken(), vec!["opened", "closed"]);
}

#[test]
fn an_installed_application_hands_out_what_its_plugins_provided() {
    let mut app = GuineaApp::new();
    app.plugin(Settings);
    let runtime = app.start().expect("install");

    let store = runtime.service::<Store>().map(|store| store.0);

    assert_eq!(store, Some("db"));
}

mod exports {
    use guinea_core::scope::Scope;
    use guinea_macros::{feature, installs, reducer};

    use super::super::Harness;
    use super::{FeatureBuilder, GuineaApp, Plugin, PluginBuilder, builder};
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

    fn localised() -> GuineaApp {
        let mut app = GuineaApp::new();
        app.install::<Localisation>();
        app
    }

    struct Forgetful;

    impl AppFeature for Forgetful {
        type Exports = (Language,);

        fn install(_app: &mut FeatureBuilder) -> anyhow::Result<Self> {
            Ok(Forgetful)
        }
    }

    struct Translations;

    impl Plugin for Translations {
        const ID: &'static str = "translations";
        type Exports = (Language,);

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            app.state::<Language>().seed(Language("de")).plain();
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
        harness.app(localised()).unwrap();

        let page = harness.child();

        assert_eq!(language(harness.segment().context().scope), Some("en"));
        assert_eq!(language(page.context().scope), Some("en"));
    }

    #[test]
    fn a_window_reads_what_the_installed_application_exports() {
        let runtime = localised().start().expect("install");

        let window = crate::feature::FeatureHost::under(&runtime.context());

        assert_eq!(language(window.scope()), Some("en"));
    }

    #[test]
    fn a_window_sits_under_the_application_it_was_opened_from_not_the_latest_one() {
        let english = localised().start().expect("install");
        let mut german = GuineaApp::new();
        german.plugin(Translations);
        let _german = german.start().expect("install");

        let window = crate::feature::FeatureHost::under(&english.context());

        assert_eq!(language(window.scope()), Some("en"));
        assert_eq!(window.application().map(|app| app.scope), Some(english.context().scope));
    }

    #[test]
    fn a_feature_reads_what_the_application_exports() {
        let mut harness = Harness::new(0);
        harness.app(localised()).unwrap();

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
        harness.plugin(Translations).unwrap();

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
    use guinea_macros::{actor, feature, handler, installs};

    use crate::observability::{app_actors, installed_plugins};
    use super::{FeatureBuilder, GuineaApp, Plugin, PluginBuilder};

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
        app.plugin(Tools)?;
        app.spawn(Sweeper);
        Ok(Housekeeping)
    }

    fn housekeeping_app() -> GuineaApp {
        let mut app = GuineaApp::new();
        app.install::<Housekeeping>();
        app
    }

    struct Tools;

    impl Plugin for Tools {
        type Exports = ();

        const ID: &'static str = "test.tools";

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            app.spawn(Loose);
            Ok(())
        }
    }

    #[test]
    fn an_application_actor_names_the_feature_that_spawned_it() {
        let runtime = housekeeping_app().start().expect("install");
        let app = runtime.context().scope;

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
        harness.app(housekeeping_app()).unwrap();
        let app = harness.application().scope;

        assert_eq!(installed_plugins(app), ["test.tools"]);
        assert_eq!(app_actors(app).len(), 2);
    }

    #[test]
    fn two_applications_on_one_thread_are_read_apart() {
        let mut first = crate::app::Harness::new(0);
        first.app(housekeeping_app()).unwrap();
        let second = crate::app::Harness::new(0);

        assert_eq!(installed_plugins(first.application().scope), ["test.tools"]);
        assert_eq!(installed_plugins(second.application().scope), [""; 0]);
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
