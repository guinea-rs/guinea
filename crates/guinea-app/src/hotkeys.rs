//! System-wide shortcuts owned by a scope, behind `global-hotkeys`.
//!
//! Enable `guinea/global-hotkeys`; import `guinea::hotkeys::{HotkeyBinding,
//! HotKey, Modifiers, Code}`. The same key types support serde for application
//! settings. `HotkeyBinding::follows(settings.changing())` follows a
//! [`Changing`] value containing `Option<HotKey>`; `None` disables it.
//!
//! `cx.global_hotkey(binding, |event| TogglePanel { shortcut: event.shortcut,
//! state: event.state })?` sends a typed message to this feature's handler.
//! Registration belongs to the scope, not the returned handle. The handle's
//! `status` feed reports requested/effective bindings and rebinding errors.
//! A conflict preserves the last working binding. Subscribe to status when
//! showing an editor for shortcuts: later errors cannot return through `?`.
//!
//! Requires the toolkit's native event loop: Windows, macOS (main thread),
//! or Linux/X11. This does not implement Wayland portals or local UI shortcuts.
//! Guinea owns `global-hotkey`'s process-wide event handler; do not install
//! another handler or consume that crate's event receiver alongside it.

use std::{
    any::Any,
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex, Once,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
pub use global_hotkey::{
    HotKeyState,
    hotkey::{Code, HotKey, HotKeyParseError, Modifiers},
};
use guinea_core::{
    __private::Section,
    feed::{Feed, FeedWriter},
    scope::Scope,
    sources::Changing,
};

/// The binding that produced the event, captured before delivery to the UI.
/// `shortcut.mods` describes the registered combination, not extra held keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeyEvent {
    pub shortcut: HotKey,
    pub state: HotKeyState,
}

/// Registration failed. Later failures are published through the status feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyError(pub String);
impl std::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for HotkeyError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotkeyStatus {
    pub requested: Option<HotKey>,
    pub effective: Option<HotKey>,
    pub error: Option<HotkeyError>,
}

/// A non-owning view of registration status. Dropping it does not unregister.
#[derive(Clone, Copy, Debug)]
pub struct HotkeyRegistration {
    pub status: Feed<HotkeyStatus>,
}

trait BindingSource: Send + Sync {
    fn now(&self) -> Option<HotKey>;
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send>;
}
struct Following<C>(C);
impl<C: Changing<Value = Option<HotKey>>> BindingSource for Following<C> {
    fn now(&self) -> Option<HotKey> {
        self.0.now().flatten()
    }
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        self.0.watch(changed)
    }
}
struct Fixed(Option<HotKey>);
impl BindingSource for Fixed {
    fn now(&self) -> Option<HotKey> {
        self.0
    }
    fn watch(&self, _: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        Box::new(())
    }
}

/// A fixed shortcut or a reactive setting. Persistence stays in the app's store.
pub struct HotkeyBinding(Arc<dyn BindingSource>);
impl HotkeyBinding {
    pub fn follows(source: impl Changing<Value = Option<HotKey>>) -> Self {
        Self(Arc::new(Following(source)))
    }
}
impl From<HotKey> for HotkeyBinding {
    fn from(key: HotKey) -> Self {
        Some(key).into()
    }
}
impl From<Option<HotKey>> for HotkeyBinding {
    fn from(key: Option<HotKey>) -> Self {
        Self(Arc::new(Fixed(key)))
    }
}

trait Driver {
    fn register(&self, key: HotKey) -> Result<(), HotkeyError>;
    fn unregister(&self, key: HotKey) -> Result<(), HotkeyError>;
}
impl Driver for GlobalHotKeyManager {
    fn register(&self, key: HotKey) -> Result<(), HotkeyError> {
        self.register(key)
            .map_err(|error| HotkeyError(error.to_string()))
    }
    fn unregister(&self, key: HotKey) -> Result<(), HotkeyError> {
        self.unregister(key)
            .map_err(|error| HotkeyError(error.to_string()))
    }
}

struct Hub {
    driver: Box<dyn Driver>,
    native: bool,
    owned: RefCell<HashMap<u32, u64>>,
}
thread_local! {
    static HUB: RefCell<Weak<Hub>> = RefCell::new(Weak::new());
    static ENTRIES: RefCell<HashMap<u64, Weak<Entry>>> = RefCell::new(HashMap::new());
}
static NEXT: AtomicU64 = AtomicU64::new(1);
static ROUTES: Mutex<Option<HashMap<u32, Route>>> = Mutex::new(None);
static HANDLER: Once = Once::new();

struct Route {
    captured: Captured,
    awake: guinea_core::scope::Awake,
}

#[derive(Clone, Copy)]
struct Captured {
    token: u64,
    generation: u64,
    event: HotkeyEvent,
}

fn capture(event: GlobalHotKeyEvent) {
    let captured = ROUTES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|routes| routes.get(&event.id))
        .filter(|route| route.awake.now())
        .map(|route| route.captured);
    if let Some(mut captured) = captured {
        captured.event.state = event.state;
        guinea_core::ui_thread::invoke_on_ui(move || {
            if let Some(entry) = find(captured.token) {
                entry.deliver(captured);
            }
        });
    }
}
fn find(token: u64) -> Option<Rc<Entry>> {
    ENTRIES.with_borrow(|entries| entries.get(&token).and_then(Weak::upgrade))
}
fn native_hub() -> Result<Rc<Hub>, HotkeyError> {
    if guinea_core::ui_thread::on_ui_thread() != Some(true) {
        return Err(HotkeyError(
            "global shortcuts must be installed on the initialized UI thread".into(),
        ));
    }
    HUB.with(|slot| {
        if let Some(hub) = slot.borrow().upgrade() {
            return Ok(hub);
        }
        HANDLER.call_once(|| GlobalHotKeyEvent::set_event_handler(Some(capture)));
        let driver = GlobalHotKeyManager::new().map_err(|error| HotkeyError(error.to_string()))?;
        let hub = Rc::new(Hub {
            driver: Box::new(driver),
            native: true,
            owned: RefCell::default(),
        });
        *slot.borrow_mut() = Rc::downgrade(&hub);
        Ok(hub)
    })
}

struct State {
    status: HotkeyStatus,
    generation: u64,
    closed: bool,
    // Failed rollback registrations stay owned until teardown can retry them.
    retired: Vec<HotKey>,
}
struct Entry {
    token: u64,
    scope: Scope,
    hub: Rc<Hub>,
    source: HotkeyBinding,
    watching: RefCell<Option<Box<dyn Any + Send>>>,
    callback: RefCell<Option<Rc<dyn Fn(HotkeyEvent)>>>,
    state: RefCell<State>,
    status: RefCell<Option<FeedWriter<HotkeyStatus>>>,
    queued: Arc<AtomicBool>,
}

impl Entry {
    fn reserve(&self, key: HotKey) -> Result<(), HotkeyError> {
        if self.hub.owned.borrow().contains_key(&key.id()) {
            return Err(HotkeyError(format!(
                "shortcut `{key}` is already registered in this application"
            )));
        }
        self.hub.driver.register(key)?;
        self.hub.owned.borrow_mut().insert(key.id(), self.token);
        Ok(())
    }
    fn release(&self, key: HotKey) -> Result<(), HotkeyError> {
        self.hub.driver.unregister(key)?;
        self.hub.owned.borrow_mut().remove(&key.id());
        if self.hub.native {
            let mut routes = ROUTES.lock().unwrap();
            if let Some(routes) = routes.as_mut() {
                if routes
                    .get(&key.id())
                    .is_some_and(|route| route.captured.token == self.token)
                {
                    routes.remove(&key.id());
                }
            }
        }
        Ok(())
    }
    fn refresh(&self) -> Result<(), HotkeyError> {
        self.queued.store(false, Ordering::Release);
        if self.state.borrow().closed || !self.scope.is_alive() {
            return Ok(());
        }
        let requested = self.source.0.now();
        let effective = self.state.borrow().status.effective;
        let result = (|| {
            if requested == effective {
                return Ok(());
            }
            if let Some(key) = requested {
                self.reserve(key)?;
            }
            if let Some(old) = effective {
                if let Err(error) = self.release(old) {
                    if let Some(new) = requested {
                        if let Err(rollback) = self.release(new) {
                            self.state.borrow_mut().retired.push(new);
                            return Err(HotkeyError(format!(
                                "{error}; rollback also failed: {rollback}"
                            )));
                        }
                    }
                    return Err(error);
                }
            }
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.status.effective = requested;
            if self.hub.native {
                if let Some(shortcut) = requested {
                    ROUTES
                        .lock()
                        .unwrap()
                        .get_or_insert_with(HashMap::new)
                        .insert(
                            shortcut.id(),
                            Route {
                                captured: Captured {
                                    token: self.token,
                                    generation: state.generation,
                                    event: HotkeyEvent {
                                        shortcut,
                                        state: HotKeyState::Pressed,
                                    },
                                },
                                awake: self.scope.awake(),
                            },
                        );
                }
            }
            Ok(())
        })();
        let status = {
            let mut state = self.state.borrow_mut();
            state.status.requested = requested;
            state.status.error = result.clone().err();
            state.status.clone()
        };
        if let Some(writer) = self.status.borrow().as_ref() {
            writer.set(status);
        }
        result
    }
    fn deliver(&self, captured: Captured) {
        let state = self.state.borrow();
        if state.closed
            || !self.scope.is_alive()
            || !self.scope.is_awake()
            || captured.token != self.token
            || captured.generation != state.generation
            || state.status.effective != Some(captured.event.shortcut)
        {
            return;
        }
        drop(state);
        // Take a clone out: handlers may synchronously remove their own scope.
        let callback = self.callback.borrow().as_ref().map(Rc::clone);
        if let Some(callback) = callback {
            callback(captured.event);
        }
    }
    fn invalidate_pending(&self) {
        let mut state = self.state.borrow_mut();
        state.generation += 1;
        if self.hub.native {
            if let Some(routes) = ROUTES.lock().unwrap().as_mut() {
                for route in routes
                    .values_mut()
                    .filter(|route| route.captured.token == self.token)
                {
                    route.captured.generation = state.generation;
                }
            }
        }
    }
    fn close(&self) {
        let (effective, retired) = {
            let mut state = self.state.borrow_mut();
            if state.closed {
                return;
            }
            state.closed = true;
            (
                state.status.effective.take(),
                std::mem::take(&mut state.retired),
            )
        };
        self.watching.borrow_mut().take();
        self.callback.borrow_mut().take();
        ENTRIES.with_borrow_mut(|entries| {
            entries.remove(&self.token);
        });
        for key in effective.into_iter().chain(retired) {
            if let Err(error) = self.release(key) {
                tracing::error!(shortcut = %key, %error, "unregistering global shortcut");
            }
        }
        if self.hub.native {
            if let Some(routes) = ROUTES.lock().unwrap().as_mut() {
                routes.retain(|_, route| route.captured.token != self.token);
            }
        }
    }
}

#[track_caller]
pub(crate) fn install(
    section: Section,
    binding: HotkeyBinding,
    callback: impl Fn(HotkeyEvent) + 'static,
) -> Result<HotkeyRegistration, HotkeyError> {
    install_with(section, binding, callback, native_hub()?)
}
#[track_caller]
fn install_with(
    section: Section,
    source: HotkeyBinding,
    callback: impl Fn(HotkeyEvent) + 'static,
    hub: Rc<Hub>,
) -> Result<HotkeyRegistration, HotkeyError> {
    let scope = section.scope();
    assert!(
        scope.is_alive(),
        "registering a shortcut in a removed scope"
    );
    let token = NEXT.fetch_add(1, Ordering::Relaxed);
    let entry = Rc::new(Entry {
        token,
        scope,
        hub,
        source,
        status: RefCell::default(),
        watching: RefCell::default(),
        callback: RefCell::new(Some(Rc::new(callback))),
        state: RefCell::new(State {
            status: HotkeyStatus::default(),
            generation: 0,
            closed: false,
            retired: Vec::new(),
        }),
        queued: Arc::default(),
    });
    ENTRIES.with_borrow_mut(|entries| {
        entries.insert(token, Rc::downgrade(&entry));
    });
    let queued = entry.queued.clone();
    // Subscribe before reading, so a setting changed during install is not lost.
    *entry.watching.borrow_mut() = Some(entry.source.0.watch(Box::new(move || {
        if !queued.swap(true, Ordering::AcqRel) {
            guinea_core::ui_thread::invoke_on_ui(move || {
                if let Some(entry) = find(token) {
                    let _ = entry.refresh();
                }
            });
        }
    })));
    if let Err(error) = entry.refresh() {
        entry.close();
        return Err(error);
    }
    let status = section.feed();
    status.set(entry.state.borrow().status.clone());
    let handle = HotkeyRegistration {
        status: status.feed(),
    };
    *entry.status.borrow_mut() = Some(status);
    let sleeping = Rc::downgrade(&entry);
    scope.on_sleep(move || {
        if let Some(entry) = sleeping.upgrade() {
            entry.invalidate_pending();
        }
    });
    scope.on_teardown(move || entry.close());
    Ok(handle)
}

#[cfg(test)]
#[path = "hotkeys/tests/book_hotkeys.rs"]
mod book_hotkeys;
#[cfg(test)]
mod tests;
