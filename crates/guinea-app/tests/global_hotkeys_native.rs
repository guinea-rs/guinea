#![cfg(all(feature = "global-hotkeys", target_os = "windows"))]

use global_hotkey::GlobalHotKeyManager;
use guinea_app::{
    feature::ScopeContext,
    hotkeys::{Code, HotKey, Modifiers},
};
use guinea_core::{scope::ScopeTree, ui_thread::UiLoop};

/// Reserves only a deliberately unusual combination; never injects input,
/// opens a visible window, or touches an application's existing registration.
#[test]
#[ignore = "requires a Windows desktop for native RegisterHotKey"]
fn native_registration_conflict_and_scope_cleanup() {
    let _ui = UiLoop::install();
    let key = HotKey::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
        Code::F24,
    );
    let independent = GlobalHotKeyManager::new().unwrap();
    let tree = ScopeTree::new();
    let cx = ScopeContext::new(tree.scope());
    cx.answers(|_: ()| {});
    let binding = cx.global_hotkey(key, |_| ()).unwrap();
    assert_eq!(binding.status.get().unwrap().effective, Some(key));
    assert!(
        independent.register(key).is_err(),
        "the OS must reserve this combination"
    );
    tree.scope().remove();
    independent
        .register(key)
        .expect("teardown must release the native reservation");
    independent.unregister(key).unwrap();
    assert!(binding.status.get().is_none());
}
