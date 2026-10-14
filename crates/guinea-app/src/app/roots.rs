//! Which roots are open.
//!
//! A *root* is one instance of an application's tree: its own [`FeatureHost`],
//! its own event bus, and - when there is routing - its own router and stack
//! of scopes. What is above it is the application: plugins, services, the
//! application's scope and the bus of its tree.
//!
//! That is the split guinea has always had; opening a second window is what
//! makes it visible. `bootstrap` guards installation per process, because
//! installing twice would open the store's database again, while the router is
//! created "one per component instance, not one per process" - so the second
//! window shares the services and gets scopes of its own.
//!
//! Deliberately not called a window. A terminal has one root and no window at
//! all; Slint's window is a component; Tauri has two levels, since one window
//! can hold several webviews and a root belongs to the webview. A window is
//! what a shell puts a root in, and guinea does not need to know.
//!
//! Kept in the root's own scope: a root is open for as long as that scope is
//! there, and goes with it.
//!
//! [`FeatureHost`]: crate::feature::FeatureHost

use std::sync::atomic::{AtomicU64, Ordering};

use guinea_core::scope::Scope;

/// Names one root for as long as it is open.
///
/// Ids are never reused, so a stale one is a closed root rather than someone
/// else's - which is what a plugin remembering a window's size needs.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RootId(u64);

impl RootId {
    /// The number behind the id, for something outside the process to name it.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for RootId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "root#{}", self.0)
    }
}

/// What a root's own scope keeps: which root it is, and what it is called.
struct Root {
    id: RootId,
    label: Option<String>,
}

impl Drop for Root {
    fn drop(&mut self) {
        tracing::debug!(id = %self.id, "root closed");
    }
}

/// Opens a root on `window`, its own scope, under a new id. It is open for as
/// long as `window` is there.
pub(crate) fn open(window: Scope) -> RootId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = RootId(NEXT.fetch_add(1, Ordering::Relaxed));

    window.keep(Root { id, label: None });
    tracing::debug!(%id, "root opened");
    id
}

/// The root's own scope `scope` is in: `scope` or the nearest above it that
/// opened one. `None` for a scope under no root - the application's own.
fn own(scope: Scope) -> Option<Scope> {
    std::iter::successors(Some(scope), Scope::parent).find(Scope::keeps::<Root>)
}

/// The root `scope` is in. `None` for a scope under no root - the
/// application's own - and for one that is gone.
pub fn of(scope: Scope) -> Option<RootId> {
    own(scope)?.kept::<Root, _>(|root| root.id)
}

/// What the root `scope` is in is called, if the shell named it.
///
/// An id lasts one run; a name lasts as long as the application does. Anything
/// that remembers something *about* a window between runs - where it was, how
/// big it was - has to key on the name, because tomorrow's ids are new.
pub fn label(scope: Scope) -> Option<String> {
    own(scope)?.kept::<Root, _>(|root| root.label.clone()).flatten()
}

/// Names the root `scope` is in: `"main"`, `"settings"`, `"log"`.
///
/// The shell's call - it is the one that knows which window this is.
///
/// # Panics
///
/// When `scope` is in no root.
pub fn set_label(scope: Scope, label: impl Into<String>) {
    let Some(own) = own(scope) else {
        panic!("naming the root {scope:?} is in, and it is in none");
    };

    let label = label.into();
    own.kept::<Root, _>(|root| root.label = Some(label));
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_core::scope::ScopeTree;

    #[test]
    fn a_scope_is_in_the_root_whose_scope_it_is_under() {
        let app = ScopeTree::new();
        let window = app.child();
        let id = open(window);
        let page = window.child().child();

        assert_eq!(of(page), Some(id));
        assert_eq!(of(window), Some(id));
        assert_eq!(of(app.scope()), None, "the application is in no root");
    }

    #[test]
    fn a_root_goes_with_its_scope() {
        let app = ScopeTree::new();
        let window = app.child();
        let page = window.child();
        open(window);

        window.remove();

        assert_eq!((of(window), of(page)), (None, None));
    }

    #[test]
    fn a_root_is_named_from_anywhere_in_it() {
        let app = ScopeTree::new();
        let window = app.child();
        let page = window.child();
        open(window);

        set_label(page, "main");

        assert_eq!(label(window).as_deref(), Some("main"));
        assert_eq!(label(page).as_deref(), Some("main"));
        assert_eq!(label(app.scope()), None, "the application is in no root");
    }

    #[test]
    fn an_unnamed_root_has_no_name() {
        let app = ScopeTree::new();
        let window = app.child();
        open(window);

        assert_eq!(label(window), None);
    }

    #[test]
    fn two_windows_are_two_roots_and_ids_are_not_reused() {
        let app = ScopeTree::new();
        let first = app.child();
        let gone = open(first);
        first.remove();

        let second = open(app.child());
        let third = open(app.child());

        assert_ne!(second, gone, "a plugin holding an id must never be handed someone else's root");
        assert_ne!(second, third);
    }
}
