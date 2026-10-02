//! Guards on the way in.
//!
//! The asymmetry with leaving is what decides where each is declared. On the
//! way out the scope exists, so a guard can read its own state - which is what
//! "unsaved changes" is - and that is why leave guards register during
//! `install`. On the way in there is nothing to read yet: the guard has to
//! answer *before* anything is installed, or a refused navigation would leave
//! a half-built tree behind. So an enter guard is part of the route
//! declaration, and what it may read is what the application holds rather than
//! what the route does.
//!
//! ```ignore
//! struct RequiresAdmin;
//!
//! impl Enter for RequiresAdmin {
//!     fn decide(cx: &EnterCx<'_>) -> Verdict {
//!         match cx.try_require::<Session>() {
//!             Some(session) if session.is_admin() => Verdict::Allow,
//!             _ => Verdict::Block,
//!         }
//!     }
//! }
//! ```
//!
//! ```ignore
//! routes! {
//!     Route {
//!         layout(AdminArea) guard(RequiresAdmin) {
//!             page(Audit) link("/admin/audit")
//!             page(Public) !guard(RequiresAdmin) link("/admin/public")
//!         }
//!     }
//! }
//! ```
//!
//! `guard` cascades, because it tightens; opting out has to name what it
//! opens, so that removing protection reads as removing protection.

use std::marker::PhantomData;
use std::sync::Arc;

use guinea_app::services::Services;
use guinea_core::SharedState;
use guinea_core::guard::Verdict;

/// Asked before a route is entered.
///
/// Returns a value rather than a future, so the overwhelming majority of
/// navigations stay exactly as synchronous as they were: [`Verdict::Allow`]
/// allocates nothing. "Optionally async" is the right to return
/// [`Verdict::Ask`] and settle its token later.
pub trait Enter: 'static {
    fn decide(cx: &EnterCx<'_>) -> Verdict;
}

/// What an enter guard is handed.
///
/// Deliberately thin. There is no scope to read - that is the whole reason
/// this runs where it does - and no route parameters either: a guard answers
/// whether this area may be entered at all, and a guard that needed the
/// parameters would be asking a question the page itself is better placed to
/// ask once it exists.
pub struct EnterCx<'a> {
    services: &'a SharedState,
    route: &'a str,
}

impl<'a> EnterCx<'a> {
    pub fn new(services: &'a SharedState, route: &'a str) -> Self {
        Self { services, route }
    }

    /// A service a plugin provided at startup, or `None` when nothing did.
    ///
    /// `None` rather than an error: a guard's answer to "there is no session
    /// service" is its own to make, and for most guards it is [`Verdict::Block`].
    #[deprecated(
        since = "0.18.9",
        note = "`try_require`, as every other context names it; in 0.19 `require` returns a Result here too"
    )]
    pub fn require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services.get::<T>()
    }

    /// A service a plugin provided at startup, or `None` when nothing did.
    ///
    /// `None` rather than an error: a guard's answer to "there is no session
    /// service" is its own to make, and for most guards it is [`Verdict::Block`].
    pub fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services.get::<T>()
    }

    /// Where the navigation was heading, for a log line that says what was
    /// refused.
    pub fn route(&self) -> &str {
        self.route
    }
}

impl Services for EnterCx<'_> {
    fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        EnterCx::try_require(self)
    }
}

/// The vtable behind a declared guard.
///
/// A trait object for the same reason [`Mount`](crate::router::Mount) is one:
/// a route's guard list is `&'static`, built by `routes!` in a `const`, and
/// `Enter::decide` is an associated function with no value to point at. The
/// marker supplies one.
pub trait EnterGuard: 'static {
    /// What the guard is called, for tracing and for the deep-link manifest.
    fn name(&self) -> &'static str;

    fn decide(&self, cx: &EnterCx<'_>) -> Verdict;
}

/// What `routes!` puts in a guard list: a zero-sized marker per guard type.
pub struct Stands<G>(pub PhantomData<G>);

impl<G: Enter> EnterGuard for Stands<G> {
    fn name(&self) -> &'static str {
        let full = std::any::type_name::<G>();
        match full.rsplit_once("::") {
            Some((_, last)) => last,
            None => full,
        }
    }

    fn decide(&self, cx: &EnterCx<'_>) -> Verdict {
        G::decide(cx)
    }
}
