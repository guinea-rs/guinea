//! What an install function takes besides its context: the services the
//! application provided, handed out by clone, and the feeds the segments
//! above it opened.
//!
//! `#[installs]`, `#[page]` and `#[layout]` write each argument after the context as
//! `(&&Given::<T>::new()).take(..)`. Method lookup tries the receiver as it
//! is, then borrowed again, then dereferenced, and the first trait with an
//! impl for that type wins: [`Twice`] on `&Given<T>` takes an
//! `Option<Feed<_>>`, [`Once`] on `&&Given<T>` a `Feed<_>` or an `Option` of
//! a service, and [`Bare`] on `Given<T>` the service itself. The choice is
//! made on the real type, an alias included, and none of the traits is
//! implemented for anything the caller wrote.

use std::any::TypeId;
use std::marker::PhantomData;
use std::ops::Deref;

use guinea_core::feed::{self, Feed};
use guinea_core::scope::Scope;
use guinea_core::short_type_name;

use super::FeatureInitContext;

/// What a feature is installed with: its context, and what the segment
/// handed in.
///
/// ```ignore
/// #[installs]
/// fn agent_link(cx: &Install<AgentLinkParams>, store: Store) -> anyhow::Result<AgentLink> {
///     let address = &cx.params().address;
///     ...
/// }
/// ```
pub struct Install<'a, P: ?Sized = ()> {
    cx: &'a FeatureInitContext,
    params: &'a P,
}

impl<'a, P: ?Sized> Install<'a, P> {
    #[doc(hidden)]
    pub fn new(cx: &'a FeatureInitContext, params: &'a P) -> Self {
        Self { cx, params }
    }

    /// What the segment handed in.
    pub fn params(&self) -> &'a P {
        self.params
    }
}

impl<P: ?Sized> Deref for Install<'_, P> {
    type Target = FeatureInitContext;

    fn deref(&self) -> &FeatureInitContext {
        self.cx
    }
}

/// A context the application's services, and the feeds above it, are
/// reached through: the scope it installs into.
pub trait Context {
    fn scope(&self) -> Scope;
}

pub fn scope<C: Context + ?Sized>(cx: &C) -> Scope {
    cx.scope()
}

/// Something a function takes and cannot do without: a service, or a feed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Need {
    pub id: TypeId,
    /// The service, or what the feed carries.
    pub name: &'static str,
    /// A feed from a segment above, rather than a service.
    pub fed: bool,
}

impl Need {
    pub fn of<T: 'static>() -> Self {
        Self {
            id: TypeId::of::<T>(),
            name: short_type_name::<T>(),
            fed: false,
        }
    }

    pub fn feed<T: 'static>() -> Self {
        Self {
            id: TypeId::of::<T>(),
            name: short_type_name::<T>(),
            fed: true,
        }
    }

    /// What the function takes, as its signature says it.
    pub fn taken(&self) -> String {
        match self.fed {
            true => format!("Feed<{}>", self.name),
            false => self.name.to_string(),
        }
    }

    fn is_there(&self, scope: Scope) -> bool {
        match self.fed {
            false => scope.provides(self.id),
            true => feed::opened_above(scope, self.id),
        }
    }
}

/// Refuses `W` when anything it cannot do without is not there, naming all
/// of it at once.
pub fn check<W: ?Sized>(scope: Scope, needs: &[Option<Need>]) -> anyhow::Result<()> {
    let missing = |fed: bool| -> Vec<String> {
        needs
            .iter()
            .flatten()
            .filter(|need| need.fed == fed && !need.is_there(scope))
            .map(|need| format!("`{}`", need.taken()))
            .collect()
    };
    let services = missing(false);
    let feeds = missing(true);

    let who = short_type_name::<W>();
    let mut refused = Vec::new();
    if !services.is_empty() {
        refused.push(format!(
            "`{who}` takes what nothing provided: {} - provide each from a plugin or the \
             application before `{who}` is installed",
            services.join(", "),
        ));
    }
    if !feeds.is_empty() {
        refused.push(format!(
            "`{who}` follows what no segment above it feeds: {} - a feed is taken from the \
             segments `{who}` is installed under, and handed on by value within one",
            feeds.join(", "),
        ));
    }

    if refused.is_empty() {
        return Ok(());
    }
    anyhow::bail!("{}", refused.join("; "))
}

/// What a service has to be to be handed out of the context.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is handed out of the application's services by clone",
    label = "taken here",
    note = "derive `Clone` for it, or provide an `Rc` or an `Arc` of it and take that"
)]
pub trait Service: Clone + 'static {}

impl<T: Clone + 'static> Service for T {}

pub struct Given<T>(PhantomData<fn() -> T>);

impl<T> Given<T> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

/// The first a lookup tries: two wrappers deep.
pub trait Twice {
    type Taken;

    fn need(&self) -> Option<Need>;
    fn take(&self, scope: Scope) -> Self::Taken;
}

impl<T: Send + Sync + 'static> Twice for &Given<Option<Feed<T>>> {
    type Taken = Option<Feed<T>>;

    fn need(&self) -> Option<Need> {
        None
    }

    fn take(&self, scope: Scope) -> Option<Feed<T>> {
        Feed::above(scope)
    }
}

/// The second: one wrapper deep.
pub trait Once {
    type Taken;

    fn need(&self) -> Option<Need>;
    fn take(&self, scope: Scope) -> Self::Taken;
}

impl<T: Send + Sync + 'static> Once for &&Given<Feed<T>> {
    type Taken = Feed<T>;

    fn need(&self) -> Option<Need> {
        Some(Need::feed::<T>())
    }

    fn take(&self, scope: Scope) -> Feed<T> {
        Feed::above(scope).expect("checked before it was taken")
    }
}

impl<T: Service> Once for &&Given<Option<T>> {
    type Taken = Option<T>;

    fn need(&self) -> Option<Need> {
        None
    }

    fn take(&self, scope: Scope) -> Option<T> {
        scope.service::<T>()
    }
}

/// The last: a service as it is.
pub trait Bare {
    type Taken;

    fn need(&self) -> Option<Need>;
    fn take(&self, scope: Scope) -> Self::Taken;
}

impl<T: Service> Bare for Given<T> {
    type Taken = T;

    fn need(&self) -> Option<Need> {
        Some(Need::of::<T>())
    }

    fn take(&self, scope: Scope) -> T {
        scope.service::<T>().expect("checked before it was taken")
    }
}
