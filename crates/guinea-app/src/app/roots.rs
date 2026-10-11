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
//! [`FeatureHost`]: crate::feature::FeatureHost

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use guinea_core::actor::event_bus::Event;
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

/// A root was opened.
#[derive(Clone, Copy, Debug)]
pub struct RootOpened(pub RootId);

/// A root was closed, and its scopes are gone with it.
#[derive(Clone, Copy, Debug)]
pub struct RootClosed(pub RootId);

impl Event for RootOpened {}
impl Event for RootClosed {}

fn open_roots() -> &'static Mutex<Vec<Open>> {
    static OPEN: OnceLock<Mutex<Vec<Open>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Clone, Debug)]
struct Open {
    id: RootId,
    label: Option<String>,
}

/// The roots that are open, oldest first.
pub fn roots() -> Vec<RootId> {
    open_roots()
        .lock()
        .map(|open| open.iter().map(|root| root.id).collect())
        .unwrap_or_default()
}

/// What this root is called, if the shell named it.
///
/// An id lasts one run; a name lasts as long as the application does. Anything
/// that remembers something *about* a window between runs - where it was, how
/// big it was - has to key on the name, because tomorrow's ids are new.
pub fn label(id: RootId) -> Option<String> {
    let open = open_roots().lock().ok()?;
    open.iter()
        .find(|root| root.id == id)
        .and_then(|root| root.label.clone())
}

/// The open root called `label`, if there is one.
pub fn labelled(label: &str) -> Option<RootId> {
    let open = open_roots().lock().ok()?;
    open.iter()
        .find(|root| root.label.as_deref() == Some(label))
        .map(|root| root.id)
}

/// Names a root, for whoever opened it: `"main"`, `"settings"`, `"log"`.
///
/// The shell's call - it is the one that knows which window this is. Naming
/// two open roots the same is allowed but pointless: [`labelled`] then answers
/// with the older one.
pub fn set_label(id: RootId, label: impl Into<String>) {
    let label = label.into();
    if let Ok(mut open) = open_roots().lock()
        && let Some(root) = open.iter_mut().find(|root| root.id == id)
    {
        root.label = Some(label);
    }
}

/// How many roots are open. `0` before the first one, and again after the
/// last one closes.
pub fn count() -> usize {
    open_roots().lock().map(|open| open.len()).unwrap_or(0)
}

pub fn is_open(id: RootId) -> bool {
    open_roots()
        .lock()
        .map(|open| open.iter().any(|root| root.id == id))
        .unwrap_or(false)
}

/// A root's place in the registry, held by whoever owns the root.
///
/// Closing is a drop: the host goes, the scopes under it go, and the entry
/// goes with them. Nothing has to remember to deregister, which is the only
/// way a registry stays true.
#[derive(Debug)]
pub struct Registration {
    id: RootId,
    within: Scope,
}

impl Registration {
    /// A root opened `within` a scope: its opening and closing are published
    /// on that scope's bus.
    pub(crate) fn open(within: Scope) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = RootId(NEXT.fetch_add(1, Ordering::Relaxed));

        if let Ok(mut open) = open_roots().lock() {
            open.push(Open { id, label: None });
        }

        tracing::debug!(%id, roots = count(), "root opened");
        within.publish(RootOpened(id));

        Self { id, within }
    }

    pub fn id(&self) -> RootId {
        self.id
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut open) = open_roots().lock() {
            open.retain(|root| root.id != self.id);
        }

        tracing::debug!(id = %self.id, roots = count(), "root closed");
        self.within.publish(RootClosed(self.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_core::scope::ScopeTree;

    #[test]
    fn a_root_is_open_while_its_registration_lives() {
        let app = ScopeTree::new();
        let root = Registration::open(app.scope());
        let id = root.id();

        assert!(is_open(id));
        assert!(roots().contains(&id));

        drop(root);
        assert!(!is_open(id), "closing is a drop, with nothing to remember");
    }

    #[test]
    fn a_name_outlives_the_id_it_was_given_to() {
        // What a plugin restoring a window's size needs: today's id is new,
        // yesterday's name is the same.
        let app = ScopeTree::new();
        let first = Registration::open(app.scope());
        set_label(first.id(), "main");
        assert_eq!(label(first.id()).as_deref(), Some("main"));
        assert_eq!(labelled("main"), Some(first.id()));
        drop(first);

        assert_eq!(labelled("main"), None, "a name is only for an open root");

        let again = Registration::open(app.scope());
        set_label(again.id(), "main");
        assert_eq!(labelled("main"), Some(again.id()));
    }

    #[test]
    fn an_unnamed_root_has_no_name() {
        let app = ScopeTree::new();
        let root = Registration::open(app.scope());
        assert_eq!(label(root.id()), None);
    }

    #[test]
    fn ids_are_not_reused() {
        let app = ScopeTree::new();
        let first = Registration::open(app.scope());
        let id = first.id();
        drop(first);

        let second = Registration::open(app.scope());

        assert_ne!(
            second.id(),
            id,
            "a plugin holding an id must never be handed someone else's root"
        );
    }

    #[test]
    fn roots_are_listed_side_by_side_and_leave_one_at_a_time() {
        // Counted by identity and not by how many there are: the registry is
        // process-wide, and the other tests in this binary open roots of their
        // own on other threads.
        let app = ScopeTree::new();
        let one = Registration::open(app.scope());
        let two = Registration::open(app.scope());
        let (first, second) = (one.id(), two.id());

        let open = roots();
        assert!(open.contains(&first) && open.contains(&second));

        drop(one);
        assert!(!roots().contains(&first));
        assert!(is_open(second), "closing one root leaves the other alone");
        drop(two);
    }
}
