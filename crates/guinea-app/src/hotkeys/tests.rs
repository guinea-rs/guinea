use super::*;
use guinea_core::scope::ScopeTree;
use std::{cell::Cell, collections::HashSet};

#[derive(Default)]
pub(super) struct Fake {
    pub keys: RefCell<HashSet<HotKey>>,
    pub operations: RefCell<Vec<(bool, HotKey)>>,
    pub fail_release: Cell<Option<HotKey>>,
}
impl Driver for Rc<Fake> {
    fn register(&self, key: HotKey) -> Result<(), HotkeyError> {
        self.operations.borrow_mut().push((true, key));
        if !self.keys.borrow_mut().insert(key) {
            return Err(HotkeyError(format!("conflict: {key}")));
        }
        Ok(())
    }
    fn unregister(&self, key: HotKey) -> Result<(), HotkeyError> {
        self.operations.borrow_mut().push((false, key));
        if self.fail_release.get() == Some(key) {
            return Err(HotkeyError(format!("release failed: {key}")));
        }
        assert!(self.keys.borrow_mut().remove(&key));
        Ok(())
    }
}
pub(super) fn fake_hub() -> (Rc<Hub>, Rc<Fake>) {
    let fake = Rc::new(Fake::default());
    (
        Rc::new(Hub {
            driver: Box::new(fake.clone()),
            owned: RefCell::default(),
            native: false,
        }),
        fake,
    )
}
pub(super) fn key(code: Code) -> HotKey {
    HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), code)
}

#[derive(Clone, Default)]
pub(super) struct Setting(pub Arc<Mutex<SettingData>>);
#[derive(Default)]
pub(super) struct SettingData {
    pub value: Option<HotKey>,
    next: u64,
    pub watchers: HashMap<u64, Arc<dyn Fn() + Send + Sync>>,
}
struct Watching(Setting, u64);
impl Drop for Watching {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().watchers.remove(&self.1);
    }
}
impl Changing for Setting {
    type Value = Option<HotKey>;
    fn now(&self) -> Option<Self::Value> {
        Some(self.0.lock().unwrap().value)
    }
    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        let mut data = self.0.lock().unwrap();
        data.next += 1;
        let id = data.next;
        data.watchers.insert(id, Arc::from(changed));
        Box::new(Watching(self.clone(), id))
    }
}
impl Setting {
    pub fn set(&self, value: Option<HotKey>) {
        let watchers = {
            let mut data = self.0.lock().unwrap();
            data.value = value;
            data.watchers.values().cloned().collect::<Vec<_>>()
        };
        for changed in watchers {
            changed();
        }
    }
}

struct Fixture {
    tree: ScopeTree,
    fake: Rc<Fake>,
    source: Setting,
    entry: Rc<Entry>,
    seen: Rc<RefCell<Vec<HotkeyEvent>>>,
    handle: HotkeyRegistration,
}
impl Fixture {
    fn new() -> Self {
        let tree = ScopeTree::new();
        let (hub, fake) = fake_hub();
        let source = Setting::default();
        source.0.lock().unwrap().value = Some(key(Code::KeyP));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let recorded = seen.clone();
        let handle = install_with(
            tree.scope().section(),
            HotkeyBinding::follows(source.clone()),
            move |event| recorded.borrow_mut().push(event),
            hub,
        )
        .unwrap();
        let entry = ENTRIES.with_borrow(|entries| {
            entries
                .values()
                .filter_map(Weak::upgrade)
                .find(|entry| entry.scope == tree.scope())
                .unwrap()
        });
        Self {
            tree,
            fake,
            source,
            entry,
            seen,
            handle,
        }
    }
    fn change(&self, value: Option<HotKey>) -> Result<(), HotkeyError> {
        self.source.0.lock().unwrap().value = value;
        self.entry.refresh()
    }
    fn captured(&self) -> Captured {
        Captured {
            token: self.entry.token,
            generation: self.entry.state.borrow().generation,
            event: HotkeyEvent {
                shortcut: self.entry.state.borrow().status.effective.unwrap(),
                state: HotKeyState::Pressed,
            },
        }
    }
}

#[test]
fn rebind_registers_first_and_publishes_status() {
    let f = Fixture::new();
    let new = key(Code::KeyQ);
    f.change(Some(new)).unwrap();
    assert_eq!(
        *f.fake.operations.borrow(),
        [
            (true, key(Code::KeyP)),
            (true, new),
            (false, key(Code::KeyP))
        ]
    );
    assert_eq!(
        *f.handle.status.get().unwrap(),
        HotkeyStatus {
            requested: Some(new),
            effective: Some(new),
            error: None
        }
    );
}

#[test]
fn conflict_keeps_old_binding_and_a_later_change_recovers() {
    let f = Fixture::new();
    let conflict = key(Code::KeyQ);
    f.fake.keys.borrow_mut().insert(conflict);
    let error = f.change(Some(conflict)).unwrap_err();
    assert_eq!(
        *f.handle.status.get().unwrap(),
        HotkeyStatus {
            requested: Some(conflict),
            effective: Some(key(Code::KeyP)),
            error: Some(error)
        }
    );
    f.entry.deliver(f.captured());
    assert_eq!(f.seen.borrow().len(), 1);
    f.change(Some(key(Code::KeyR))).unwrap();
    assert!(f.handle.status.get().unwrap().error.is_none());
}

#[test]
fn old_captured_events_are_rejected_even_after_binding_returns() {
    let f = Fixture::new();
    let old = f.captured();
    f.change(Some(key(Code::KeyQ))).unwrap();
    f.change(Some(key(Code::KeyP))).unwrap();
    f.entry.deliver(old);
    assert!(f.seen.borrow().is_empty());
    f.entry.deliver(f.captured());
    assert_eq!(f.seen.borrow().as_slice(), &[old.event]);
}

#[test]
fn none_disables_and_same_setting_does_not_reregister() {
    let f = Fixture::new();
    f.change(Some(key(Code::KeyP))).unwrap();
    assert_eq!(f.fake.operations.borrow().len(), 1);
    let old = f.captured();
    f.change(None).unwrap();
    assert!(f.fake.keys.borrow().is_empty());
    f.entry.deliver(old);
    assert!(f.seen.borrow().is_empty());
    assert_eq!(*f.handle.status.get().unwrap(), HotkeyStatus::default());
}

#[test]
fn sleeping_scope_keeps_reservation_but_drops_events() {
    let f = Fixture::new();
    let pending = f.captured();
    f.tree.scope().sleep();
    f.entry.deliver(f.captured());
    assert!(f.seen.borrow().is_empty());
    assert_eq!(f.fake.keys.borrow().len(), 1);
    f.tree.scope().wake();
    f.entry.deliver(pending);
    assert!(f.seen.borrow().is_empty());
    f.entry.deliver(f.captured());
    assert_eq!(f.seen.borrow().len(), 1);
}

#[test]
fn teardown_releases_key_subscription_and_callback_despite_retained_handle() {
    let f = Fixture::new();
    let old = f.captured();
    assert_eq!(Rc::strong_count(&f.seen), 2);
    assert_eq!(f.source.0.lock().unwrap().watchers.len(), 1);
    f.tree.scope().remove();
    assert!(f.fake.keys.borrow().is_empty());
    assert!(f.source.0.lock().unwrap().watchers.is_empty());
    assert_eq!(Rc::strong_count(&f.seen), 1);
    assert!(find(f.entry.token).is_none());
    f.entry.deliver(old);
    assert!(f.seen.borrow().is_empty());
    assert!(f.handle.status.get().is_none());
}

#[test]
fn failed_old_unregister_rolls_new_registration_back() {
    let f = Fixture::new();
    f.fake.fail_release.set(Some(key(Code::KeyP)));
    f.change(Some(key(Code::KeyQ))).unwrap_err();
    assert_eq!(*f.fake.keys.borrow(), HashSet::from([key(Code::KeyP)]));
    assert_eq!(
        f.handle.status.get().unwrap().effective,
        Some(key(Code::KeyP))
    );
    f.fake.fail_release.set(None);
}

#[test]
fn duplicate_binding_is_rejected_and_does_not_unsubscribe_first_owner() {
    let f = Fixture::new();
    let other = f.tree.scope().child();
    assert!(
        install_with(
            other.section(),
            key(Code::KeyP).into(),
            |_| {},
            f.entry.hub.clone()
        )
        .is_err()
    );
    assert_eq!(f.fake.operations.borrow().len(), 1);
    f.entry.deliver(f.captured());
    assert_eq!(f.seen.borrow().len(), 1);
    other.remove();
    assert_eq!(f.fake.keys.borrow().len(), 1);
}

#[test]
fn scope_may_remove_itself_in_its_handler() {
    let tree = ScopeTree::new();
    let scope = tree.scope();
    let (hub, fake) = fake_hub();
    install_with(
        scope.section(),
        key(Code::KeyP).into(),
        move |_| scope.remove(),
        hub,
    )
    .unwrap();
    let entry = ENTRIES.with_borrow(|entries| {
        entries
            .values()
            .filter_map(Weak::upgrade)
            .find(|entry| entry.scope == scope)
            .unwrap()
    });
    entry.deliver(Captured {
        token: entry.token,
        generation: 1,
        event: HotkeyEvent {
            shortcut: key(Code::KeyP),
            state: HotKeyState::Pressed,
        },
    });
    assert!(!scope.is_alive());
    assert!(fake.keys.borrow().is_empty());
}

#[test]
fn repeated_rebinding_keeps_one_reservation() {
    let f = Fixture::new();
    for _ in 0..500 {
        f.change(Some(key(Code::KeyQ))).unwrap();
        f.change(Some(key(Code::KeyP))).unwrap();
        assert_eq!(f.entry.hub.owned.borrow().len(), 1);
        assert_eq!(f.fake.keys.borrow().len(), 1);
        assert_eq!(f.source.0.lock().unwrap().watchers.len(), 1);
    }
    f.tree.scope().remove();
    assert!(f.entry.hub.owned.borrow().is_empty());
}

#[test]
fn failed_initial_registration_drops_subscription_and_handler() {
    let tree = ScopeTree::new();
    let (hub, fake) = fake_hub();
    let source = Setting::default();
    source.0.lock().unwrap().value = Some(key(Code::KeyP));
    fake.keys.borrow_mut().insert(key(Code::KeyP));
    let held = Rc::new(());
    let captured = held.clone();
    assert!(
        install_with(
            tree.scope().section(),
            HotkeyBinding::follows(source.clone()),
            move |_| {
                let _ = &captured;
            },
            hub.clone()
        )
        .is_err()
    );
    assert_eq!(Rc::strong_count(&held), 1);
    assert!(source.0.lock().unwrap().watchers.is_empty());
    assert!(hub.owned.borrow().is_empty());
    assert!(!ENTRIES.with_borrow(|entries| {
        entries
            .values()
            .filter_map(Weak::upgrade)
            .any(|entry| entry.scope == tree.scope())
    }));
}
