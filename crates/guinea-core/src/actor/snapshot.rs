use crate::actor::shape::Shape;
use crate::inspect::Node;

/// An actor as devtools read it. See [`Scope::actors`](crate::scope::Scope::actors).
pub struct ActorSnapshot {
    pub id: usize,
    pub type_name: &'static str,
    /// Its state's preview; [`Scope::inspect_actor`](crate::scope::Scope::inspect_actor)
    /// opens it.
    pub state: Node,
    pub shape: Shape,
    pub owner: Owner,
}

/// Where an actor lives, for grouping it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    /// The scope that owns it.
    pub scope: Option<crate::scope::ScopeId>,
    /// The feature that created it.
    pub feature: Option<&'static str>,
    /// The reducer it drives, when it was created with `driven_by`.
    pub drives: Option<&'static str>,
}
