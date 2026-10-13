//! What an install function takes besides its context: the services the
//! application provided, handed out by clone.
//!
//! `#[installs]` and `#[given]` write each argument after the context as
//! `(&Given::<T>::new()).take(..)`. Method lookup tries the receiver as it is,
//! `&Given<T>`, before it borrows it again, so `Given<Option<T>>` finds
//! [`Optional`] first and every other `T` reaches [`Required`] on
//! `&Given<T>`. The choice is made on the real type, an alias included, and
//! neither trait is implemented for anything the caller wrote.

use std::any::TypeId;
use std::marker::PhantomData;
use std::ops::Deref;

use guinea_core::{SharedState, short_type_name};

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

/// A context the application's services are reached through.
pub trait Context {
    fn provided(&self) -> &SharedState;
}

pub fn services<C: Context + ?Sized>(cx: &C) -> &SharedState {
    cx.provided()
}

/// A service a function takes and cannot do without.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Need {
    pub id: TypeId,
    pub name: &'static str,
}

impl Need {
    pub fn of<T: 'static>() -> Self {
        Self {
            id: TypeId::of::<T>(),
            name: short_type_name::<T>(),
        }
    }
}

/// Refuses `W` when anything it cannot do without was not provided, naming
/// all of it at once.
pub fn check<W: ?Sized>(services: &SharedState, needs: &[Option<Need>]) -> anyhow::Result<()> {
    let missing: Vec<String> = needs
        .iter()
        .flatten()
        .filter(|need| !services.has(need.id))
        .map(|need| format!("`{}`", need.name))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    let who = short_type_name::<W>();
    anyhow::bail!(
        "`{who}` takes what nothing provided: {} - provide each from a plugin or the \
         application before `{who}` is installed",
        missing.join(", "),
    )
}

/// What a service has to be to be handed out of the context.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is handed out of the application's services by clone",
    label = "taken here",
    note = "derive `Clone` for it, or provide an `Arc` of it and take that"
)]
pub trait Service: Clone + Send + Sync + 'static {}

impl<T: Clone + Send + Sync + 'static> Service for T {}

pub struct Given<T>(PhantomData<fn() -> T>);

impl<T> Given<T> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

pub trait Optional {
    type Taken;

    fn need(&self) -> Option<Need>;
    fn take(&self, services: &SharedState) -> Self::Taken;
}

impl<T: Service> Optional for Given<Option<T>> {
    type Taken = Option<T>;

    fn need(&self) -> Option<Need> {
        None
    }

    fn take(&self, services: &SharedState) -> Option<T> {
        services.get::<T>()
    }
}

pub trait Required {
    type Taken;

    fn need(&self) -> Option<Need>;
    fn take(&self, services: &SharedState) -> Self::Taken;
}

impl<T: Service> Required for &Given<T> {
    type Taken = T;

    fn need(&self) -> Option<Need> {
        Some(Need::of::<T>())
    }

    fn take(&self, services: &SharedState) -> T {
        services.get::<T>().expect("checked before it was taken")
    }
}
