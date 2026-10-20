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

use std::marker::PhantomData;
use std::ops::Deref;

use guinea_core::__private::opened_above;
use guinea_core::feed::Feed;
use guinea_core::scope::Scope;

use super::FeatureInitContext;

/// What a feature is installed with: its context, and what the segment
/// handed in.
///
/// ```
/// # use guinea_app::app::roots::Root;
/// # use guinea_app::feature::Install;
/// # use guinea_macros::{feature, installs, reducer};
/// # #[derive(Clone)]
/// # pub struct Store(&'static str);
/// # pub struct AgentLinkParams {
/// #     pub address: String,
/// # }
/// # #[derive(Clone, Default, Debug, PartialEq)]
/// # pub struct Linked(pub String);
/// # #[reducer]
/// # fn linked(this: &mut Linked, to: String) {
/// #     this.0 = to;
/// # }
/// # feature! { pub AgentLink { exports { Linked } } }
/// #[installs]
/// fn agent_link(cx: &Install<AgentLinkParams>, store: Store) -> anyhow::Result<AgentLink> {
///     let address = &cx.params().address;
///
///     let linked = cx
///         .state::<Linked>()
///         .seed(Linked(format!("{address}, remembered in {}", store.0)))
///         .plain();
///     Ok(AgentLink(linked))
/// }
/// # fn main() -> anyhow::Result<()> {
/// # let (host, ()) = guinea_app::app::start("doc", |app| {
/// #     guinea_macros::plugin! { Resources { provides { Store } } }
/// #     #[guinea_macros::builds]
/// #     fn resources(_app: &mut guinea_app::app::PluginBuilder, value: Store) -> anyhow::Result<Resources> { Ok(Resources(value)) }
/// #     app.plugin::<Resources>(Store("agents.db"))?;
/// #     Ok(())
/// # })?;
/// # let window = Root::under(&host.context());
/// # let _page = window.install(|cx| {
/// #     cx.install::<AgentLink>(&AgentLinkParams { address: "10.0.0.7".into() })?;
/// #     let linked = cx.read::<Linked>().map(|linked| linked.0.clone());
/// #     assert_eq!(linked.as_deref(), Some("10.0.0.7, remembered in agents.db"));
/// #     Ok(())
/// # })?;
/// # Ok(())
/// # }
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

/// Where a function's arguments are looked up: the scope its context
/// installs into, which nothing outside this crate reads.
#[derive(Clone, Copy)]
pub struct Lookup(pub(crate) Scope);

/// A context the application's services, and the feeds above it, are
/// reached through.
pub trait Context {
    fn lookup(&self) -> Lookup;
}

pub fn lookup<C: Context + ?Sized>(cx: &C) -> Lookup {
    cx.lookup()
}

pub fn publish_dependency<Dependency: Clone + 'static>(
    cx: &super::ScopeContext,
    value: Dependency,
    by: &'static str,
) {
    if let Err(first) = guinea_core::__private::publish_dependency(cx.scope, value, by) {
        use crate::diagnostic::Diagnostic;
        let report = Diagnostic::new("Invalid guinea composition")
            .with_related(vec![
                Diagnostic::new(format!("service `{}`", std::any::type_name::<Dependency>()))
                    .with_related(vec![
                        Diagnostic::new(format!("provided by `{first}`")),
                        Diagnostic::new(format!("provided again by `{by}`")),
                    ]),
            ])
            .render();
        panic!("{report}");
    }
}

pub fn check_inputs<W: ?Sized>(
    Lookup(scope): Lookup,
    inputs: &[super::declarations::Port],
    source: guinea_core::actor::shape::Declared,
) {
    check_named(std::any::type_name::<W>(), scope, inputs, source);
}

pub(crate) fn check_named(
    who: &str,
    scope: Scope,
    inputs: &[super::declarations::Port],
    source: guinea_core::actor::shape::Declared,
) {
    use super::declarations::{Channel, in_scope, provider_diagnostic};
    use crate::diagnostic::Diagnostic;
    let declarations = in_scope(scope);
    let mut failures = Vec::new();
    for input in inputs {
        let available = match input.channel {
            Channel::Service => guinea_core::__private::has_dependency(scope, input.id),
            Channel::Feed => opened_above(scope, input.id),
            Channel::State => false,
        };
        if available {
            continue;
        }
        let providers: Vec<_> = declarations
            .iter()
            .filter(|declaration| {
                declaration
                    .outputs
                    .iter()
                    .any(|output| output.id == input.id && output.channel == input.channel)
            })
            .collect();
        if input.optional && providers.is_empty() {
            continue;
        }
        let details = if providers.is_empty() {
            vec![Diagnostic::new("no declared provider")]
        } else {
            providers.into_iter().map(|provider| {
                provider_diagnostic(provider).with_related(vec![
                    Diagnostic::new("declared, but not available in this scope; check installation order and scope ancestry")
                ])
            }).collect()
        };
        failures.push(
            Diagnostic::new(format!(
                "{:?} `{}` is unavailable",
                input.channel, input.name
            ))
            .with_related(details),
        );
    }
    if !failures.is_empty() {
        let report = Diagnostic::new("Invalid guinea composition")
            .with_related(vec![
                Diagnostic::new(format!(
                    "{who} at {}:{}:{}",
                    source.file.replace('\\', "/"),
                    source.line,
                    source.column
                ))
                .with_related(failures),
            ])
            .render();
        panic!("{report}");
    }
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

    fn input(&self) -> super::declarations::Port;
    fn take(&self, from: Lookup) -> Self::Taken;
}

impl<T: Send + Sync + 'static> Twice for &Given<Option<Feed<T>>> {
    type Taken = Option<Feed<T>>;

    fn input(&self) -> super::declarations::Port {
        super::declarations::Port::of::<T>(super::declarations::Channel::Feed).optional()
    }

    fn take(&self, Lookup(scope): Lookup) -> Option<Feed<T>> {
        Feed::above(scope)
    }
}

/// The second: one wrapper deep.
pub trait Once {
    type Taken;

    fn input(&self) -> super::declarations::Port;
    fn take(&self, from: Lookup) -> Self::Taken;
}

impl<T: Send + Sync + 'static> Once for &&Given<Feed<T>> {
    type Taken = Feed<T>;

    fn input(&self) -> super::declarations::Port {
        super::declarations::Port::of::<T>(super::declarations::Channel::Feed)
    }

    fn take(&self, Lookup(scope): Lookup) -> Feed<T> {
        Feed::above(scope).expect("checked before it was taken")
    }
}

impl<T: Service> Once for &&Given<Option<T>> {
    type Taken = Option<T>;

    fn input(&self) -> super::declarations::Port {
        super::declarations::Port::of::<T>(super::declarations::Channel::Service).optional()
    }

    fn take(&self, Lookup(scope): Lookup) -> Option<T> {
        guinea_core::__private::dependency::<T>(scope)
    }
}

/// The last: a service as it is.
pub trait Bare {
    type Taken;

    fn input(&self) -> super::declarations::Port;
    fn take(&self, from: Lookup) -> Self::Taken;
}

impl<T: Service> Bare for Given<T> {
    type Taken = T;

    fn input(&self) -> super::declarations::Port {
        super::declarations::Port::of::<T>(super::declarations::Channel::Service)
    }

    fn take(&self, Lookup(scope): Lookup) -> T {
        guinea_core::__private::dependency::<T>(scope).expect("checked before it was taken")
    }
}
