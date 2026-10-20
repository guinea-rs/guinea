---
title: Installing plugins
sidebar:
  order: 4
---

A feature describes part of your application. A plugin packages something
several applications can use: a settings store, localisation, window geometry
or devtools. It knows guinea, but does not need to know your application's
pages or features.

Plugins are Rust code linked into the application, not modules loaded at
runtime. Add the plugin crate to your dependencies, then build it in the
application function with `app.plugin::<P>(params)?`. The plugin's parameters
and returned value describe how to connect it to the rest of the application.

This chapter uses a small plugin defined alongside the application, so its
whole contract is visible. The wiring is the same for a plugin from another
crate and does not depend on the GUI backend.

## What installation returns

`MemoryStore` builds a key-value store and returns its handle. The store is
only in memory: this example does not open a file or save settings between
runs.

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

`plugin!` declares the returned `Store` field under `provides`.
`#[builds]` implements construction: the argument after `PluginBuilder`
becomes the plugin's parameters. Here that is an initial theme.

Cloning `Store` shares one map; building another `MemoryStore` creates a
different map. The application can build both and decide which one its
pages should receive.

## The application chooses

<!-- shown: plugins: application wiring -->
```rust
fn app(app: &mut FeatureBuilder) -> anyhow::Result<Store> {
    let MemoryStore(user) = app.plugin::<MemoryStore>("dark")?;
    let MemoryStore(internal) = app.plugin::<MemoryStore>("light")?;
    app.provide(user);
    Ok(internal)
}
```
<!-- /shown -->

Each `app.plugin` call builds a new instance immediately. Installation is
not deduplicated by plugin type. Here `user` and `internal` are independent
stores, even though they have the same Rust type.

`app.provide(user)` makes the chosen handle available as the application's
`Store`. Returning a value from a plugin does **not** provide it automatically:

<!-- shown: plugins: returned is not provided -->
```rust
#[test]
fn returning_a_service_does_not_publish_it() {
    let mut app = TestApp::new();
    let MemoryStore(store) = app.plugin::<MemoryStore>("dark").unwrap();

    assert!(app.service::<Store>().is_none());
    app.provide(store);
    assert_eq!(
        app.service::<Store>().unwrap().get("theme").as_deref(),
        Some("dark")
    );
}
```
<!-- /shown -->

There is one provided service per Rust type. Providing a second `Store` is
a setup error and panics; building a second store is fine. If different
consumers need different stores, pass their handles explicitly, or give the
services distinct wrapper types.

The example's `app` returns the internal handle so the test can inspect it.
In a GUI application's function, put this wiring before constructing and
returning the backend's `Window`, as in [pages and layouts](../pages/).

## Taking a provided service

A feature declares the service it needs as an argument after its installation
context:

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

`#[installs]` looks up `Store` and passes a clone to the function. It must
already have been provided when the feature installs. For an optional service,
use `Option<Store>` instead; a missing required service is a setup error.

`Appearance` copies the stored theme into its initial state. This is not a
subscription: changing the map later does not update an installed `Theme`.
Reactive updates need an explicit subscription or another source of updates.

The harness builds the same application wiring and installs the feature in a
window scope, without opening an OS window:

<!-- shown: plugins: testing application wiring -->
```rust
#[guinea::test]
fn the_application_chooses_which_store_its_pages_receive(h: &mut Harness) {
    let internal = h.app(app).unwrap();
    h.install::<Appearance>(&()).unwrap();

    assert_eq!(h.state::<Theme>().0, "dark");
    assert_eq!(internal.get("theme").as_deref(), Some("light"));

    internal.set("theme", "blue");
    let page = h.child();
    page.install::<Appearance>(&()).unwrap();
    assert_eq!(page.state::<Theme>().0, "dark");
}
```
<!-- /shown -->

The child installs a fresh `Appearance` after the internal store changes.
It still receives the user store. The test checks service selection, not
merely the value previously copied into the parent's state.

## Connecting plugins

When one plugin needs another's service, build the producer first and pass
the returned handle in the consumer's parameters. The application chooses
the instance and order explicitly; guinea does not resolve a dependency
graph or install dependencies implicitly.

Two declarations serve different purposes:

- `provides { Store }` puts a service handle in the plugin's returned value.
  The application decides where to pass it and whether to call `provide`.
- `exports { SomeState }` makes reducer state claimed by the plugin readable
  from windows and their descendants after successful installation. This
  follows the state lookup described in [scopes](../scopes/).

A service is not automatically reducer state, and a returned service is not
automatically an application-wide dependency.

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
