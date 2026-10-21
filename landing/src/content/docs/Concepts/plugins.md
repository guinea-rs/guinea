---
title: Installing plugins
sidebar:
  order: 4
---

A plugin packages something several applications can use: settings,
localisation, window geometry or devtools. It knows guinea, but not your
application's pages. Plugins are Rust code linked into the application,
not modules loaded at runtime.

## Declare the outputs

For the shared model of inputs, outputs and scope visibility, see
[dependencies and application wiring](../dependencies/).

This in-memory store is a complete example of a plugin. `provides` lists
the dependency it returns:

<!-- shown: plugins: a plugin and its service -->
```rust
#[derive(Clone, Debug, Default)]
pub struct Store(Rc<RefCell<BTreeMap<String, String>>>);

impl Store {
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.borrow().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: &str) {
        self.0.borrow_mut().insert(key.to_owned(), value.to_owned());
    }
}

plugin! {
    pub MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder, theme: &'static str) -> anyhow::Result<MemoryStore> {
    let store = Store::default();
    store.set("theme", theme);
    Ok(MemoryStore(store))
}
```
<!-- /shown -->

The first argument after `PluginBuilder` is the plugin's explicit
configuration. Any arguments after that are dependencies supplied by the
framework. Use `()` as configuration when a plugin needs dependencies
but has no configuration of its own.

## Install the plugin

<!-- shown: plugins: application wiring -->
```rust
fn app(app: &mut FeatureBuilder) -> anyhow::Result<Store> {
    let MemoryStore(user) = app.plugin::<MemoryStore>("dark")?;
    Ok(user)
}
```
<!-- /shown -->

After a successful build, guinea automatically publishes the outputs
declared in `provides`. No manual registration or service lookup is needed.
Cloning `Store` shares the same map. A second value of the same service
type on one scope branch is a composition bug: use distinct wrapper types
for distinct stores.

<!-- shown: plugins: declared services are published -->
```rust
#[guinea::test]
fn declared_services_are_published_automatically(h: &mut Harness) {
    h.plugin::<MemoryStore>("dark").unwrap();
    h.install::<Appearance>(&()).unwrap();
    assert_eq!(h.state::<Theme>().0, "dark");
}
```
<!-- /shown -->

## Receive dependencies as arguments

A feature declares its dependencies after the installation context:

<!-- shown: plugins: a feature takes a service -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Theme(pub String);

#[reducer]
fn theme(this: &mut Theme, value: String) {
    this.0 = value;
}

feature! {
    pub Appearance {
        exports { Theme }
    }
}

#[installs]
fn appearance(cx: &Install, store: Store) -> anyhow::Result<Appearance> {
    let theme = cx
        .state::<Theme>()
        .seed(Theme(store.get("theme").unwrap_or_default()))
        .plain();
    Ok(Appearance(theme))
}
```
<!-- /shown -->

The framework passes a clone of `Store`. The same argument-based injection
works for application features and for the `install` methods processed by
`#[page]` and `#[layout]`.

An optional dependency is written `Option<Store>`. It receives `None`
only if there is no declared provider. A declared provider that has not
made its output available is a composition error, even for an optional
input. The framework panics before entering the installer.

`Appearance` copies the current theme into state, rather than subscribing
to changes. A newly installed instance reads the current value:

<!-- shown: plugins: testing application wiring -->
```rust
#[guinea::test]
fn the_application_shares_its_declared_store_with_pages(h: &mut Harness) {
    let store = h.app(app).unwrap();
    h.install::<Appearance>(&()).unwrap();

    assert_eq!(h.state::<Theme>().0, "dark");
    assert_eq!(store.get("theme").as_deref(), Some("dark"));

    store.set("theme", "blue");
    let page = h.child();
    page.install::<Appearance>(&()).unwrap();
    assert_eq!(page.state::<Theme>().0, "blue");
}
```
<!-- /shown -->

## Application outputs

Application-level features use the same `provides` declaration and return
their outputs instead of registering values manually:

<!-- shown: plugins: application outputs -->
```rust
#[derive(Clone)]
pub struct AppName(pub &'static str);

feature! {
    pub Application {
        provides { AppName }
    }
}

#[installs]
fn application(app: &mut FeatureBuilder) -> anyhow::Result<Application> {
    app.plugin::<MemoryStore>("dark")?;
    Ok(Application(AppName("guinea")))
}
```
<!-- /shown -->

`Application` publishes `AppName`; its `MemoryStore` plugin publishes
`Store`. Installers that need either declare it as an argument.

The three output sections have different meanings:

- `provides` publishes cloneable dependency values.
- `exports` exposes reducer state to readers below the scope.
- `feeds` opens value streams for descendant segments.

A dependency belongs to the scope whose installer returned it and is
visible there and in its descendants, not in sibling scopes. Removing that
scope releases its stored dependency values. Application and plugin outputs
belong to the application scope, so all its windows can receive them.

The concrete declaration graph requires one provider declaration per service
type. The same local provider may be instantiated in separate sibling
scopes; each instance owns its own value. Dependencies are not shadowed
by another value farther down the same branch.

## Declaration validation

Macros register installer contracts through `inventory`. Before the backend
calls the application function, guinea checks the linked concrete
declarations for missing required providers, conflicting service providers,
contradictory inputs, duplicate outputs and dependency cycles. It does not
run installer bodies, open resources or infer calls inside those bodies.
Call `guinea::app::validate_declarations()` to run the same check explicitly.

Invalid contracts panic with a structured `miette` diagnostic tree and
source locations. The actual installation also checks that inputs are
available before entering each installer: a declaration is not a guarantee
that its provider has already run or that a feed is reachable from this
scope. Real failures such as opening a file still use `Result`.

Feed declarations alone do not describe a mount tree, so the initial
check cannot establish feed ancestry. Generic installers are registered
as templates; their concrete inputs and outputs are checked when that
specialization is installed. If a generic output family could supply a
missing input, the initial check defers that input rather than rejecting
a potentially valid graph. An attempted specialization remains a declared
provider even if its resource creation fails; an optional consumer cannot
silently turn that failure into `None`. Tests supply fixture values through plugins
with declared outputs, using the same installation path as application code.

## Lifetime and failure

A plugin registers its actors, timers and subscriptions in the application's
scope. Dropping the returned plugin value does not uninstall them. They live
with the application host, not with the page that happens to use them.

For resources that need explicit cleanup, the builder accepts `on_cleanup`:

<!-- shown: plugins: registering cleanup -->
```rust
plugin! {
    Cleanup {}
}

#[builds]
fn cleanup(app: &mut PluginBuilder, cleaned: Rc<Cell<bool>>) -> anyhow::Result<Cleanup> {
    app.on_cleanup(move || {
        cleaned.set(true);
        Ok(())
    });
    Ok(Cleanup)
}
```
<!-- /shown -->

Here the flag only makes the cleanup observable in a test:

<!-- shown: plugins: cleanup belongs to the application -->
```rust
#[test]
fn a_plugins_cleanup_runs_when_the_application_ends() {
    let cleaned = Rc::new(Cell::new(false));
    let mut app = TestApp::new();
    app.plugin::<Cleanup>(cleaned.clone()).unwrap();

    assert!(!cleaned.get());
    drop(app);
    assert!(cleaned.get());
}
```
<!-- /shown -->

Cleanups run in reverse registration order. An error returned by a cleanup
is logged.

### When installation fails halfway through

Suppose an indexing plugin creates a lock file, registers its deletion at
shutdown, starts a polling timer and writes a startup log. It then tries to
read `index.toml`. If that file is missing, installation returns an error,
but the earlier steps have already happened.

The following deliberately orders setup this way to make a partial failure
observable. The timer increments a counter instead of doing real indexing:

<!-- shown: plugins: a partially built indexer -->
```rust
pub struct IndexingParams {
    pub directory: PathBuf,
    pub polls: Rc<Cell<u32>>,
}

plugin! {
    Indexing {}
}

#[builds]
fn indexing(app: &mut PluginBuilder, params: IndexingParams) -> anyhow::Result<Indexing> {
    let lock = params.directory.join("index.lock");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)?;
    app.on_cleanup(move || {
        std::fs::remove_file(lock)?;
        Ok(())
    });

    let polls = params.polls;
    app.repeat(Duration::from_secs(1), move || polls.set(polls.get() + 1));
    std::fs::write(params.directory.join("startup.log"), b"starting indexer\n")?;

    let _settings = std::fs::read_to_string(params.directory.join("index.toml"))?;
    Ok(Indexing)
}
```
<!-- /shown -->

If the caller catches the error and keeps the application alive, the timer
continues and the lock file remains. The registered cleanup belongs to the
scope, not to the failed function call:

<!-- shown: plugins: catching an error keeps the scope -->
```rust
#[guinea::test]
fn catching_an_indexer_error_leaves_its_timer_running(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let polls = Rc::new(Cell::new(0));
    let mut app = TestApp::new();
    let result = app.plugin::<Indexing>(IndexingParams {
        directory: directory.path().to_owned(),
        polls: polls.clone(),
    });

    assert!(result.is_err());
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 3);
    assert!(directory.path().join("index.lock").exists());

    drop(app);
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 3);
    assert!(!directory.path().join("index.lock").exists());
    assert!(directory.path().join("startup.log").exists());
}
```
<!-- /shown -->

Dropping the application stops the timer and runs the lock-file cleanup.
It does not remove `startup.log`: nothing registered an action to delete it.
Neither a failed installation nor shutdown automatically reverses writes
to the filesystem.

For required plugins, let the error escape startup through `?`. The next
test uses `TestApp` to model the ownership: if setup fails, the local
application is dropped rather than returned to the caller.

<!-- shown: plugins: propagating an error drops the host -->
```rust
#[guinea::test]
fn an_indexer_error_propagated_out_of_startup_releases_the_scope(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let polls = Rc::new(Cell::new(0));

    let result = (|| -> anyhow::Result<TestApp> {
        let mut app = TestApp::new();
        app.plugin::<Indexing>(IndexingParams {
            directory: directory.path().to_owned(),
            polls: polls.clone(),
        })?;
        Ok(app)
    })();

    assert!(result.is_err());
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 0);
    assert!(!directory.path().join("index.lock").exists());
    assert!(directory.path().join("startup.log").exists());
}
```
<!-- /shown -->

A backend's `run` likewise owns the host while calling the application
function. If that function returns an error, the host is dropped, along
with everything installed before the error.

In production, validate settings before starting work wherever possible.
If a plugin is optional and startup must continue without it, arrange for
its partial setup to be cleaned up before returning an error. Do not rely
on `app.plugin` to roll it back: a failed build publishes no exports and
withdraws its claims and action routing, but resources already created in
the scope remain until the scope is removed.

Reusable plugins live in the separate
[guinea-plugins repository](https://github.com/guinea-rs/guinea-plugins).
Use a plugin version compatible with the guinea version your application
builds against.
