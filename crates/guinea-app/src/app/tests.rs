use std::cell::RefCell;
use std::rc::Rc;

use guinea_core::actor::shape::name;
use guinea_macros::{builds, feature, installs, plugin};

use super::roots::Root;
use super::{FeatureBuilder, GuineaHost, PluginBuilder, TestApp, start};

thread_local! {
    static TRACE: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn trace(step: &'static str) {
    TRACE.with(|t| t.borrow_mut().push(step));
}

fn taken() -> Vec<&'static str> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

fn builder() -> TestApp {
    TRACE.with(|t| t.borrow_mut().clear());
    TestApp::new()
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
    app.plugin::<Settings>("db")?;
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
fn an_application_goes_when_its_host_is_let_go_of() {
    taken();
    let (host, _) = start("test", |app| app.plugin::<Tidy>(())).unwrap();
    assert_eq!(taken(), [""; 0]);

    drop(host);

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
fn a_plugin_cannot_publish_the_same_service_twice() {
    let mut app = builder();

    app.plugin::<Settings>("settings").unwrap();
    let duplicate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.plugin::<Settings>("state")));
    assert!(duplicate.is_err());
    assert_eq!(taken(), ["settings"]);
}

#[test]
fn what_a_plugin_declares_is_published_automatically() {
    let mut app = builder();

    app.plugin::<Settings>("settings").unwrap();
    assert_eq!(guinea_core::__private::dependency::<Store>(app.scope), Some(Store("settings")));
}

#[test]
fn every_copy_of_a_plugin_is_listed() {
    let mut app = builder();
    let _ = app.plugin::<Tidy>(());
    let _ = app.plugin::<Tidy>(());

    assert_eq!(
        app.scope.kept::<super::registry::Plugins, _>(|plugins| plugins.ids()),
        Some(vec![name::<Tidy>(), name::<Tidy>()])
    );
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

    assert_eq!(guinea_core::__private::dependency::<Store>(app.scope).map(|store| store.0), Some("db"));
}

#[test]
fn subscriptions_taken_during_install_are_dropped_on_shutdown() {
    use guinea_core::actor::event_bus::Event;

    #[derive(Clone)]
    struct Tick;
    impl Event for Tick {}

    let host = GuineaHost::new();
    let app = PluginBuilder::new(host.scope());
    let bus = app.scope.bus();
    let seen = Rc::new(RefCell::new(0usize));

    {
        let seen = seen.clone();
        app.subscribe::<Tick>(move |_| *seen.borrow_mut() += 1);
    }

    assert_eq!(bus.count_subscribers::<Tick>(), 1);

    drop(app);
    drop(host);

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
    let (host, ()) = start("test", |app| {
        app.plugin::<Greets>(())?;
        Ok(())
    })
    .expect("install");

    let window = Root::under(&host.context());
    let scope = window
        .install(|ctx| {
            assert_eq!(guinea_core::__private::dependency::<Greeting>(ctx.scope).map(|greeting| greeting.0), Some("hello"));
            ctx.subscribe(|_: Ping| trace("pinged"));
            Ok(())
        })
        .expect("install the feature");

    window.scope().bus().publish(Ping);
    assert_eq!(taken(), vec!["pinged"]);

    drop(scope);
    window.scope().bus().publish(Ping);
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
        let (host, ()) = start("test", localised).expect("install");

        let window = crate::app::roots::Root::under(&host.context());

        assert_eq!(language(window.scope()), Some("en"));
    }

    #[test]
    fn a_window_sits_under_the_application_it_was_opened_from_not_the_latest_one() {
        let (english, ()) = start("test", localised).expect("install");
        let _german = start("test", |app| app.plugin::<Translations>(())).expect("install");

        let window = crate::app::roots::Root::under(&english.context());

        assert_eq!(language(window.scope()), Some("en"));
        assert!(window.application() == &english.context());
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

mod observed {
    use guinea_core::actor::In;
    use guinea_core::actor::shape::name;
    use guinea_core::observability::{Event, Watching};
    use guinea_core::observability::panels::{self, Panel};
    use guinea_macros::{actor, builds, feature, handler, installs, plugin, reducer};

    use super::{FeatureBuilder, PluginBuilder, start};
    use crate::app::roots::{self, Root};
    use crate::app::{GuineaHost, Observed};

    pub struct Sweep;

    #[derive(Debug, Default)]
    pub struct Sweeper;

    actor! {
        Sweeper {
            handlers { Sweep }
        }
    }

    #[handler]
    fn sweep(_: In<Sweeper, Sweep>) {}

    #[derive(Debug, Default)]
    pub struct Loose;

    actor! {
        Loose {
            handlers { Sweep }
        }
    }

    #[handler]
    fn loose(_: In<Loose, Sweep>) {}

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

    plugin! {
        Watches {
            provides { Observed }
        }
    }

    #[builds]
    fn watches(app: &mut PluginBuilder) -> anyhow::Result<Watches> {
        Ok(Watches(app.observed()))
    }

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Count(u32);

    #[reducer]
    fn count(this: &mut Count, by: u32) {
        this.0 += by;
    }

    /// An application and the tool watching it.
    struct Watched {
        watching: Watching,
        observed: Observed,
        host: GuineaHost,
    }

    fn watched(backend: &'static str, app: fn(&mut FeatureBuilder) -> anyhow::Result<()>) -> Watched {
        let (host, observed) = start(backend, |builder| {
            app(builder)?;
            let Watches(observed) = builder.plugin::<Watches>(())?;
            Ok(observed)
        })
        .expect("install");
        let watching = observed.watch(|| {}).expect("nothing else watches");

        Watched {
            watching,
            observed,
            host,
        }
    }

    #[test]
    fn a_tool_is_told_what_draws_the_application_and_what_it_was_built_with() {
        let watched = watched("egui", housekeeping_app);

        let events = watched.watching.take().events;

        let built: Vec<&Event> = events
            .iter()
            .filter(|event| matches!(event, Event::Backend { .. } | Event::Plugin { .. }))
            .collect();
        assert_eq!(
            built,
            [
                &Event::Backend { name: "egui" },
                &Event::Plugin { name: name::<Tools>() },
                &Event::Plugin { name: name::<Watches>() },
            ]
        );
    }

    #[test]
    fn an_application_actor_is_told_with_the_feature_that_spawned_it_and_read_when_asked() {
        let watched = watched("test", housekeeping_app);

        let events = watched.watching.take().events;

        let added = |named: &str| {
            events.iter().find_map(|event| match event {
                Event::ActorAdded { id, type_name, owner } if type_name.ends_with(named) => {
                    Some((*id, owner.feature))
                }
                _ => None,
            })
        };
        let read = added("Sweeper")
            .and_then(|(id, _)| watched.observed.actor(id))
            .map(|actor| actor.type_name);
        assert_eq!(
            (added("Sweeper").map(|(_, feature)| feature), added("Loose").map(|(_, feature)| feature)),
            (Some(Some(name::<Housekeeping>())), Some(None)),
            "a plugin is not a feature"
        );
        assert!(read.is_some_and(|read| read.ends_with("Sweeper")), "{read:?}");
    }

    #[test]
    fn a_state_is_read_when_the_tool_asks() {
        let watched = watched("test", |app| {
            app.state::<Count>().seed(Count(3)).plain();
            Ok(())
        });
        watched.watching.take();
        let app = watched.host.context().__scope().id();

        let read = watched
            .observed
            .state(app, std::any::type_name::<Count>())
            .map(|state| state.state.preview);

        assert_eq!(read.as_deref(), Some("Count(3)"));
    }

    #[test]
    fn a_root_that_opens_is_named_and_closes_is_told() {
        let watched = watched("test", |_| Ok(()));
        watched.watching.take();

        let window = Root::under(&watched.host.context());
        roots::set_label(window.scope(), "main");
        let (root, scope) = (window.id().get(), window.scope().id());
        drop(window);

        let events = watched.watching.take().events;
        let rooted: Vec<&Event> = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::RootOpened { .. } | Event::Labelled { .. } | Event::RootClosed { .. }
                )
            })
            .collect();
        assert_eq!(
            rooted,
            [
                &Event::RootOpened { root, scope },
                &Event::Labelled { root, label: "main".into() },
                &Event::RootClosed { root },
            ]
        );
    }

    #[test]
    fn a_tool_that_starts_watching_is_told_the_roots_open_and_their_names() {
        let (host, observed) = start("test", |app| {
            let Watches(observed) = app.plugin::<Watches>(())?;
            Ok(observed)
        })
        .expect("install");
        let window = Root::under(&host.context());
        roots::set_label(window.scope(), "main");

        let watching = observed.watch(|| {}).expect("nothing else watches");
        let events = watching.take().events;

        let root = window.id().get();
        assert!(
            events.contains(&Event::RootOpened { root, scope: window.scope().id() })
                && events.contains(&Event::Labelled { root, label: "main".into() }),
            "{events:#?}"
        );
    }

    #[test]
    fn what_another_application_on_the_thread_does_is_not_told() {
        let watched = watched("test", |_| Ok(()));
        watched.watching.take();

        let (_other, ()) = start("test", housekeeping_app).expect("install");

        assert_eq!(watched.watching.take().events, []);
    }

    #[test]
    fn what_the_backend_offers_for_the_application_is_read_when_asked() {
        let watched = watched("test", |_| Ok(()));
        let _offered = panels::contribute_to_app(|| {
            Some(Panel {
                id: "backend",
                title: "Backend",
                nodes: Vec::new(),
            })
        });

        let ids: Vec<&str> = watched.observed.panels(None).iter().map(|panel| panel.id).collect();

        assert_eq!(ids, ["backend"]);
    }
}

mod renders {
    use std::cell::RefCell;
    use std::rc::Rc;

    use guinea_core::observability::Rendering;
    use guinea_core::trace::{self, Point, Trace};

    use crate::app::{Harness, TestApp};

    #[test]
    fn what_a_thread_records_outside_any_actor_goes_to_the_application_it_runs() {
        let app = TestApp::new();
        let other = TestApp::new();
        let heard = |app: &TestApp| {
            let seen = Rc::new(RefCell::new(Vec::new()));
            let noted = seen.clone();
            let observing = trace::sink_of(app.scope).expect("the application is there").observe(
                move |trace| {
                    if let Trace::Mark(record) = trace {
                        noted.borrow_mut().push(record.id);
                    }
                },
            );
            (seen, observing)
        };
        let (app_seen, _app) = heard(&app);
        let (other_seen, _other) = heard(&other);

        let marked = trace::mark(|| Point::Plugin {
            kind: "outside",
            fields: Vec::new(),
        });

        assert_eq!(
            (app_seen.borrow().clone(), other_seen.borrow().clone()),
            (vec![], vec![marked]),
            "the newest application on the thread takes what nothing nearer claims"
        );
    }

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
