//! Which roots are open.
//!
//! A *root* is one window's part of the application's tree: a scope of its
//! own under the application's, and - when there is routing - its own
//! navigator and stack of scopes. There is one tree per application, held by
//! its [`GuineaHost`]: its root is the application's scope, where the plugins,
//! the services and the application's features are, and every root sits
//! under it. The bus and the trace are the tree's, so every root shares them.
//!
//! Opening a second window is what makes the split visible: the second
//! window shares the services and gets scopes of its own.
//!
//! Deliberately not called a window. A terminal has one root and no window at
//! all; Slint's window is a component; Tauri has two levels, since one window
//! can hold several webviews and a root belongs to the webview. A window is
//! what a shell puts a root in, and guinea does not need to know.
//!
//! Which root a scope is in is kept in the root's own scope: a root is open
//! for as long as that scope is there, and goes with it.
//!
//! [`GuineaHost`]: super::GuineaHost

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use guinea_core::__private::observer;
use guinea_core::observability::Event;
use guinea_core::scope::{Scope, ScopeGuard};

use super::AppContext;
use crate::feature::{FeatureInitContext, ScopeContext};

/// A window of an application: its own scope under the application's, which
/// reads what the application's plugins provided and its features export.
///
/// Opened by a backend for each window it shows, and by a test to install
/// into. Letting go of it removes its scope and everything under it.
///
/// ```
/// use guinea_app::app::GuineaHost;
/// use guinea_app::app::roots::Root;
///
/// let host = GuineaHost::new();
/// let root = Root::under(&host.context());
///
/// assert!(root.application() == &host.context());
/// assert_eq!(guinea_app::app::roots::of(root.scope()), Some(root.id()));
/// ```
pub struct Root {
    /// Its own scope, which every scope installed through it sits under.
    /// Removed when it goes, before its registration does.
    scope: ScopeGuard,
    app: AppContext,
    id: RootId,
}

impl Root {
    /// A window of `app`: its scope under the application's, with what the
    /// application's plugins provided.
    ///
    /// `app` is what [`GuineaHost::context`](super::GuineaHost::context)
    /// hands back.
    pub fn under(app: &AppContext) -> Self {
        let scope = app.0.scope.child();
        let id = open(scope);

        Self {
            scope: scope.guard(),
            app: app.clone(),
            id,
        }
    }

    /// The application this window belongs to - what a window opened from
    /// this one is opened under.
    pub fn application(&self) -> &AppContext {
        &self.app
    }

    /// Its own scope: the root of every scope installed here.
    pub fn scope(&self) -> Scope {
        self.scope.scope()
    }

    /// Which root this is - the one a feature installed here belongs to.
    pub fn id(&self) -> RootId {
        self.id
    }

    /// The context a feature installs through, for the segment at `cursor`
    /// of its chain.
    ///
    /// ```
    /// use guinea_app::app::GuineaHost;
    /// use guinea_app::app::roots::Root;
    ///
    /// let host = GuineaHost::new();
    /// let root = Root::under(&host.context());
    /// let page = root.scope().child();
    ///
    /// let cx = root.context(page, 1);
    /// assert_eq!((cx.__scope(), cx.cursor), (page, 1));
    /// ```
    pub fn context(&self, scope: Scope, cursor: usize) -> FeatureInitContext {
        FeatureInitContext {
            scope_cx: ScopeContext::new(scope),
            cursor,
        }
    }

    /// Installs one feature into a scope of its own, right under the window's,
    /// which goes when the guard does.
    ///
    /// The whole path for an application that has no routes: no chain, no
    /// route, no backend.
    ///
    /// ```
    /// use guinea_app::app::GuineaHost;
    /// use guinea_app::app::roots::Root;
    ///
    /// let host = GuineaHost::new();
    /// let root = Root::under(&host.context());
    ///
    /// let installed = root.install(|cx| {
    ///     assert_eq!(cx.__scope().parent(), Some(root.scope()));
    ///     Ok(())
    /// })?;
    ///
    /// assert_eq!(root.scope().subtree().len(), 2);
    /// drop(installed);
    /// assert_eq!(root.scope().subtree().len(), 1);
    /// # anyhow::Ok(())
    /// ```
    pub fn install(
        &self,
        install: impl Fn(&FeatureInitContext) -> anyhow::Result<()>,
    ) -> anyhow::Result<ScopeGuard> {
        let scope = self.scope().child().guard();
        let ctx = self.context(scope.scope(), 0);
        install(&ctx)?;
        Ok(scope)
    }
}

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
struct Opened {
    id: RootId,
    /// Its own scope, which keeps it.
    scope: Scope,
    label: Option<String>,
}

thread_local! {
    static OPEN: RefCell<Vec<(RootId, Scope)>> = const { RefCell::new(Vec::new()) };
}

impl Drop for Opened {
    fn drop(&mut self) {
        let id = self.id;
        let _ = OPEN.try_with(|open| open.borrow_mut().retain(|(open, _)| *open != id));
        observer::tell(self.scope, || Event::RootClosed { root: id.0 });
        tracing::debug!(%id, "root closed");
    }
}

/// Opens a root on `window`, its own scope, under a new id. It is open for as
/// long as `window` is there.
pub(crate) fn open(window: Scope) -> RootId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = RootId(NEXT.fetch_add(1, Ordering::Relaxed));

    window.keep(Opened {
        id,
        scope: window,
        label: None,
    });
    OPEN.with(|open| open.borrow_mut().push((id, window)));
    observer::tell(window, || Event::RootOpened {
        root: id.0,
        scope: window.id(),
    });
    tracing::debug!(%id, "root opened");
    id
}

/// What a tool that starts watching is told of the root opened on `scope`,
/// if one was.
pub(crate) fn describe(scope: Scope, events: &mut Vec<Event>) {
    let told = scope.try_kept::<Opened, _>(|root| (root.id.0, root.label.clone()));
    let Some((root, label)) = told else {
        return;
    };

    events.push(Event::RootOpened {
        root,
        scope: scope.id(),
    });
    events.extend(label.map(|label| Event::Labelled { root, label }));
}

/// Every root open on this thread, in the order they opened, each with its
/// own scope.
pub fn opened() -> Vec<(RootId, Scope)> {
    OPEN.with(|open| open.borrow().clone())
}

/// The root's own scope `scope` is in: `scope` or the nearest above it that
/// opened one. `None` for a scope under no root - the application's own.
fn own(scope: Scope) -> Option<Scope> {
    std::iter::successors(Some(scope), Scope::parent).find(Scope::keeps::<Opened>)
}

/// The root `scope` is in. `None` for a scope under no root - the
/// application's own - and for one that is gone.
pub fn of(scope: Scope) -> Option<RootId> {
    own(scope)?.kept::<Opened, _>(|root| root.id)
}

/// What the root `scope` is in is called, if the shell named it.
///
/// An id lasts one run; a name lasts as long as the application does. Anything
/// that remembers something *about* a window between runs - where it was, how
/// big it was - has to key on the name, because tomorrow's ids are new.
pub fn label(scope: Scope) -> Option<String> {
    own(scope)?.kept::<Opened, _>(|root| root.label.clone()).flatten()
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
    let told = label.clone();
    let root = own.kept::<Opened, _>(|root| {
        root.label = Some(label);
        root.id.0
    });
    if let Some(root) = root {
        observer::tell(own, || Event::Labelled { root, label: told });
    }
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
    fn an_open_root_is_listed_until_its_scope_goes() {
        let app = ScopeTree::new();
        let window = app.child();
        let id = open(window);
        assert!(opened().contains(&(id, window)));

        window.remove();

        assert!(!opened().iter().any(|(root, _)| *root == id));
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

    #[derive(Debug)]
    struct Counter;

    struct Bump;

    guinea_macros::actor! {
        Counter {
            handlers {
                Bump
            }
        }
    }

    impl guinea_core::actor::Handler<Bump> for Counter {
        fn handle(_: guinea_core::actor::In<Self, Bump>) {}
    }

    #[derive(Clone)]
    struct Ping;
    impl guinea_core::actor::event_bus::Event for Ping {}

    #[test]
    fn an_actor_added_under_a_root_is_in_it() {
        let host = super::super::GuineaHost::new();
        let root = Root::under(&host.context());

        let _scope = root
            .install(|ctx| {
                ctx.spawn(Counter);
                ctx.subscribe(|_: Ping| {});
                Ok(())
            })
            .expect("installed");

        let rooted: Vec<Option<RootId>> = root
            .scope()
            .actors()
            .into_iter()
            .map(|actor| {
                actor
                    .owner
                    .scope
                    .and_then(|id| root.scope().named_by(id))
                    .and_then(of)
            })
            .collect();
        assert_eq!(rooted, [Some(root.id())]);
    }
}
