use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use guinea_app::app::{FeatureBuilder, PluginBuilder, roots::Root, start};
use guinea_app::feature::Install;
use guinea_macros::{builds, feature, installs, plugin};

#[derive(Clone)]
struct Store(Rc<Cell<u64>>);

thread_local! {
    static BUILDS: Cell<u32> = const { Cell::new(0) };
}

plugin! {
    MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder, initial: u64) -> anyhow::Result<MemoryStore> {
    BUILDS.set(BUILDS.get() + 1);
    Ok(MemoryStore(Store(Rc::new(Cell::new(initial)))))
}

feature! { ReadSettings {} }

#[installs]
fn read_settings(cx: &Install<u64>, settings: Store) -> anyhow::Result<ReadSettings> {
    assert_eq!(settings.0.get(), *cx.params());
    settings.0.set(settings.0.get() + 1);
    Ok(ReadSettings)
}

feature! { Application { provides { AppName } } }

#[derive(Clone)]
struct AppName(&'static str);

#[installs]
fn application(_app: &mut FeatureBuilder) -> anyhow::Result<Application> {
    Ok(Application(AppName("guinea")))
}

feature! { ReadName {} }

#[installs]
fn read_name(_cx: &Install, name: AppName) -> anyhow::Result<ReadName> {
    assert_eq!(name.0, "guinea");
    Ok(ReadName)
}

#[test]
fn declared_plugin_outputs_are_injected_without_registration_or_lookup() {
    let (host, MemoryStore(store)) = start("test", |app| app.plugin::<MemoryStore>(41)).unwrap();
    let root = Root::under(&host.context());
    let _page = root
        .install(|cx| {
            cx.install::<ReadSettings>(&41)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.0.get(), 42);
}

#[test]
fn application_outputs_are_injected_into_features() {
    let (host, _) = start("test", |app| app.install::<Application>()).unwrap();
    let root = Root::under(&host.context());
    let _page = root
        .install(|cx| {
            cx.install::<ReadName>(&())?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn duplicate_providers_panic_before_the_second_builder_runs() {
    BUILDS.set(0);
    let (host, _) = start("test", |app| {
        app.plugin::<MemoryStore>(41)?;
        let panic = catch_unwind(AssertUnwindSafe(|| app.plugin::<MemoryStore>(99)));
        assert!(panic.is_err());
        assert_eq!(BUILDS.get(), 1);
        Ok(())
    })
    .unwrap();
    drop(host);
}

#[derive(Clone)]
struct OptionalOnly;

feature! { WithoutOptional {} }

#[installs]
fn without_optional(_cx: &Install, value: Option<OptionalOnly>) -> anyhow::Result<WithoutOptional> {
    assert!(value.is_none());
    Ok(WithoutOptional)
}

#[test]
fn an_optional_dependency_without_any_declared_provider_is_none() {
    let (host, ()) = start("test", |_| Ok(())).unwrap();
    let root = Root::under(&host.context());
    let _page = root
        .install(|cx| {
            cx.install::<WithoutOptional>(&())?;
            Ok(())
        })
        .unwrap();
}

type StoreAlias = Store;

plugin! { DependentPlugin {} }

#[builds]
fn dependent_plugin(
    _app: &mut PluginBuilder,
    expected: u64,
    store: StoreAlias,
) -> anyhow::Result<DependentPlugin> {
    assert_eq!(store.0.get(), expected);
    Ok(DependentPlugin)
}

#[test]
fn plugins_receive_dependencies_after_their_explicit_configuration() {
    let (_host, ()) = start("test", |app| {
        app.plugin::<MemoryStore>(7)?;
        app.plugin::<DependentPlugin>(7)?;
        Ok(())
    })
    .unwrap();
}

feature! { Generic<Dependency> { provides { Rc<Dependency> } } }

#[installs]
fn generic<Dependency: Default + 'static>(
    _app: &mut FeatureBuilder,
) -> anyhow::Result<Generic<Dependency>> {
    Ok(Generic(Rc::new(Dependency::default())))
}

feature! { Observing<Dependency> { provides { Observed<Dependency> } } }

feature! { ConcreteConsumer {} }

#[installs]
fn concrete_consumer(_cx: &Install, observed: Observed<u32>) -> anyhow::Result<ConcreteConsumer> {
    assert_eq!(*observed.0, 0);
    Ok(ConcreteConsumer)
}

struct Observed<Dependency>(Rc<Dependency>);

impl<Dependency> Clone for Observed<Dependency> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[installs]
fn observing<Dependency: 'static>(
    _cx: &Install,
    dependency: Rc<Dependency>,
) -> anyhow::Result<Observing<Dependency>> {
    Ok(Observing(Observed(dependency)))
}

#[test]
fn generic_contracts_are_instantiated_with_real_type_identity() {
    let (host, (Generic(first), Generic(second))) = start("test", |app| {
        Ok((
            app.install::<Generic<u32>>()?,
            app.install::<Generic<u64>>()?,
        ))
    })
    .unwrap();
    let root = Root::under(&host.context());
    let _page = root
        .install(|cx| {
            let Observing(observed_first) = cx.install::<Observing<u32>>(&())?;
            let Observing(observed_second) = cx.install::<Observing<u64>>(&())?;
            cx.install::<ConcreteConsumer>(&())?;
            assert!(Rc::ptr_eq(&first, &observed_first.0));
            assert!(Rc::ptr_eq(&second, &observed_second.0));
            Ok(())
        })
        .unwrap();
}

#[derive(Clone)]
struct Remote;

plugin! { RemotePlugin { provides { Remote } } }

#[builds]
fn remote_plugin(_app: &mut PluginBuilder, fail: bool) -> anyhow::Result<RemotePlugin> {
    anyhow::ensure!(!fail, "connection refused");
    Ok(RemotePlugin(Remote))
}

feature! { RemoteConsumer {} }

#[installs]
fn remote_consumer(
    _app: &mut FeatureBuilder,
    remote: Option<Remote>,
) -> anyhow::Result<RemoteConsumer> {
    assert!(remote.is_some());
    Ok(RemoteConsumer)
}

#[test]
fn a_resource_failure_remains_a_result_and_publishes_nothing() {
    let (_host, ()) = start("test", |app| {
        let failure = app.plugin::<RemotePlugin>(true).err().unwrap();
        assert_eq!(failure.root_cause().to_string(), "connection refused");
        let panic = catch_unwind(AssertUnwindSafe(|| app.install::<RemoteConsumer>()))
            .err()
            .expect("a declared but unavailable optional input must panic");
        let message = panic.downcast_ref::<String>().unwrap();
        insta::assert_snapshot!(message.replace('\\', "/"));
        app.plugin::<RemotePlugin>(false)?;
        app.install::<RemoteConsumer>()?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn generic_templates_keep_their_signatures_and_manifest_outputs() {
    use guinea_app::feature::declarations::{TemplateManifest, TemplateRegistration};
    let mut signatures: Vec<_> = guinea_core::__private::inventory::iter::<TemplateRegistration>
        .into_iter()
        .map(|entry| (entry.target, entry.signature))
        .collect();
    signatures.sort();
    let mut manifests: Vec<_> = guinea_core::__private::inventory::iter::<TemplateManifest>
        .into_iter()
        .map(|entry| (entry.name, entry.exports, entry.feeds, entry.provides))
        .collect();
    manifests.sort();
    insta::assert_debug_snapshot!((signatures, manifests));
}

#[derive(Default)]
struct GenericOnly;

plugin! { GenericPlugin<Dependency> { provides { Rc<Dependency> } } }

#[builds]
fn generic_plugin<Dependency: Default + 'static>(
    _app: &mut PluginBuilder,
    fail: bool,
) -> anyhow::Result<GenericPlugin<Dependency>> {
    anyhow::ensure!(!fail, "resource unavailable");
    Ok(GenericPlugin(Rc::new(Dependency::default())))
}

feature! { OptionalGeneric {} }

#[installs]
fn optional_generic(
    _app: &mut FeatureBuilder,
    _value: Option<Rc<GenericOnly>>,
) -> anyhow::Result<OptionalGeneric> {
    Ok(OptionalGeneric)
}

#[test]
fn an_unavailable_generic_specialization_is_not_treated_as_an_absent_optional_provider() {
    let (_host, ()) = start("test", |app| {
        assert!(app.plugin::<GenericPlugin<GenericOnly>>(true).is_err());
        let panic = catch_unwind(AssertUnwindSafe(|| app.install::<OptionalGeneric>()))
            .err()
            .expect("the attempted provider is declared even though creation failed");
        let message = panic.downcast_ref::<String>().unwrap();
        insta::assert_snapshot!(message.replace('\\', "/"));
        Ok(())
    })
    .unwrap();

    let (_other_host, _) = start("other", |app| app.install::<OptionalGeneric>()).unwrap();
}

#[derive(Clone)]
struct LocalValue(u64);

feature! { LocalProvider { provides { LocalValue } } }

#[installs]
fn local_provider(cx: &Install<u64>) -> anyhow::Result<LocalProvider> {
    Ok(LocalProvider(LocalValue(*cx.params())))
}

feature! { LocalConsumer {} }

#[installs]
fn local_consumer(cx: &Install<u64>, value: Option<LocalValue>) -> anyhow::Result<LocalConsumer> {
    assert_eq!(value.expect("the dependency is available").0, *cx.params());
    Ok(LocalConsumer)
}

#[test]
fn local_outputs_do_not_escape_to_sibling_scopes() {
    let (host, ()) = start("test", |_| Ok(())).unwrap();
    let first = Root::under(&host.context());
    let second = Root::under(&host.context());
    let _first_page = first
        .install(|cx| {
            cx.install::<LocalProvider>(&1)?;
            cx.install::<LocalConsumer>(&1)?;
            Ok(())
        })
        .unwrap();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        second.install(|cx| {
            cx.install::<LocalConsumer>(&1)?;
            Ok(())
        })
    }))
    .err()
    .expect("a sibling's dependency must not be visible");
    let message = panic.downcast_ref::<String>().unwrap();
    insta::assert_snapshot!(message.replace('\\', "/"));
    let _second_page = second
        .install(|cx| {
            cx.install::<LocalProvider>(&2)?;
            cx.install::<LocalConsumer>(&2)?;
            Ok(())
        })
        .unwrap();
}
