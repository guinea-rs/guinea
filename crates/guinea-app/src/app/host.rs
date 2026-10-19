use guinea_core::scope::{Scope, ScopeTree};
use guinea_core::trace;

use super::registry::{Backend, Plugins};
use crate::feature::ScopeContext;

/// An application's tree, held on the UI thread for as long as the
/// application runs.
///
/// The application's scope is the tree's root: plugins and application
/// features are installed there, and what the application provides is
/// provided there. Each window is a [`Root`](super::roots::Root) under it,
/// opened from its [`context`](Self::context), and every window shares its
/// bus and its services. Letting go of the host tears the application down -
/// its windows first, then what it set up, the last first.
///
/// A backend's `run` gets one from [`start`](super::start); a test makes one
/// with nothing built in it, to open windows in.
///
/// ```
/// use guinea_app::app::GuineaHost;
/// use guinea_app::app::roots::Root;
///
/// let host = GuineaHost::new();
/// let main = Root::under(&host.context());
/// let settings = Root::under(&host.context());
///
/// assert_ne!(main.id(), settings.id());
/// assert_eq!(main.scope().parent(), settings.scope().parent());
/// ```
pub struct GuineaHost {
    tree: ScopeTree,
    /// What this thread records goes into the application's trace while
    /// nothing nearer is current, until after the tree is torn down.
    _recorded: trace::ThreadDefault,
}

impl GuineaHost {
    /// A tree with nothing built in it yet.
    pub fn new() -> Self {
        let tree = ScopeTree::new();
        tree.scope().keep(Plugins::default());
        tree.scope().keep(Backend::default());
        let recorded = trace::thread_default(
            &trace::sink_of(tree.scope()).expect("a tree just made is there"),
        );

        Self {
            tree,
            _recorded: recorded,
        }
    }

    /// The application, as a window is opened under it.
    ///
    /// ```
    /// use guinea_app::app::GuineaHost;
    /// use guinea_app::app::roots::Root;
    ///
    /// let host = GuineaHost::new();
    /// let window = Root::under(&host.context());
    ///
    /// assert!(window.application() == &host.context());
    /// ```
    pub fn context(&self) -> AppContext {
        AppContext(ScopeContext::new(self.tree.scope()))
    }

    pub(crate) fn scope(&self) -> Scope {
        self.tree.scope()
    }
}

impl Default for GuineaHost {
    fn default() -> Self {
        Self::new()
    }
}

/// The application, as a backend opens its windows under it: what
/// [`Root::under`](super::roots::Root::under) takes. Only whoever holds the
/// [`GuineaHost`] hands one out; nothing a feature or a plugin is handed
/// turns into it.
#[derive(Clone)]
pub struct AppContext(pub(crate) ScopeContext);

impl AppContext {
    /// The application's scope, for the adapters.
    #[doc(hidden)]
    pub fn __scope(&self) -> Scope {
        self.0.scope
    }
}

/// The same application.
impl PartialEq for AppContext {
    fn eq(&self, other: &Self) -> bool {
        self.0.scope == other.0.scope
    }
}
