use crate::actor::shape::Shape;

/// An actor as devtools read it. See [`Scope::actors`](crate::scope::Scope::actors).
pub struct ActorSnapshot {
    pub id: usize,
    pub type_name: &'static str,
    pub state: String,
    pub shape: Shape,
    pub owner: Owner,
}

/// Where an actor lives, for grouping it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    /// The scope that owns it, as `Scope::key` names it.
    pub scope: Option<usize>,
    /// The feature that created it.
    pub feature: Option<&'static str>,
    /// The reducer it drives, when it was created with `driven_by`.
    pub drives: Option<&'static str>,
}
