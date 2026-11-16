use super::tests::{Setting, fake_hub, key};
use super::*;
use crate::feature::ScopeContext;
use guinea_core::{scope::ScopeTree, ui_thread::UiLoop};

#[test]
fn reactive_shortcut_uses_the_public_api_and_routes_typed_messages() -> Result<(), HotkeyError> {
    let ui = UiLoop::install();
    let tree = ScopeTree::new();
    let cx = ScopeContext::new(tree.scope());
    let (_, fake) = fake_hub();
    // Exercise the real capture/dispatch path, using a deterministic OS driver.
    let hub = Rc::new(Hub {
        driver: Box::new(fake.clone()),
        owned: RefCell::default(),
        native: true,
    });
    HUB.with_borrow_mut(|slot| *slot = Rc::downgrade(&hub));
    let setting = Setting::default();
    setting.set(Some(key(Code::KeyP)));
    let received = Rc::new(RefCell::new(Vec::new()));
    let recorded = received.clone();
    cx.answers(move |message: TogglePanel| recorded.borrow_mut().push(message));

    //@show hotkeys: binding
    struct TogglePanel {
        shortcut: HotKey,
        state: HotKeyState,
    }

    let binding = cx.global_hotkey(HotkeyBinding::follows(setting.clone()), |event| {
        TogglePanel {
            shortcut: event.shortcut,
            state: event.state,
        }
    })?;
    //@show-end

    let entry = ENTRIES.with_borrow(|entries| {
        entries
            .values()
            .filter_map(Weak::upgrade)
            .find(|entry| entry.scope == tree.scope())
            .unwrap()
    });
    let old = Captured {
        token: entry.token,
        generation: 1,
        event: HotkeyEvent {
            shortcut: key(Code::KeyP),
            state: HotKeyState::Pressed,
        },
    };
    capture(GlobalHotKeyEvent {
        id: key(Code::KeyP).id(),
        state: HotKeyState::Pressed,
    });
    ui.drain();
    assert_eq!(received.borrow()[0].shortcut, key(Code::KeyP));
    assert_eq!(received.borrow()[0].state, HotKeyState::Pressed);

    capture(GlobalHotKeyEvent {
        id: key(Code::KeyP).id(),
        state: HotKeyState::Pressed,
    });
    tree.scope().sleep();
    capture(GlobalHotKeyEvent {
        id: key(Code::KeyP).id(),
        state: HotKeyState::Pressed,
    });
    tree.scope().wake();
    ui.drain();
    assert_eq!(
        received.borrow().len(),
        1,
        "sleep discards pending and sleeping events"
    );

    let changed = setting.clone();
    std::thread::spawn(move || {
        for _ in 0..100 {
            changed.set(Some(key(Code::KeyQ)));
        }
    })
    .join()
    .unwrap();
    ui.drain();
    assert_eq!(
        binding.status.get().unwrap().effective,
        Some(key(Code::KeyQ))
    );
    assert_eq!(
        fake.operations.borrow().len(),
        3,
        "a burst coalesces to one rebind"
    );
    entry.deliver(old);
    assert_eq!(received.borrow().len(), 1);

    setting.set(None);
    ui.drain();
    assert!(fake.keys.borrow().is_empty());
    setting.set(Some(key(Code::KeyR)));
    tree.scope().remove();
    ui.drain(); // An already queued change cannot resurrect the registration.
    assert!(fake.keys.borrow().is_empty());
    assert!(setting.0.lock().unwrap().watchers.is_empty());
    Ok(())
}
