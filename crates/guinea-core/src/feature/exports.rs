use crate::scope::{Reducer, Scope};

/// The reducers a feature lets other segments read.
///
/// `pub` for reducers: listed means a page below can read it, unlisted means
/// it is the feature's own business. A feature's internal state used to be
/// reachable from anywhere below simply because it existed, which is the
/// difference between a feature and a folder.
///
/// Implemented for `()` and for tuples of reducers, so a single export is
/// written `(Processes,)`. The trailing comma is the price of not having a
/// blanket impl over every reducer: a blanket one would overlap with the tuple
/// impls, since nothing stops a downstream crate implementing `Reducer` for a
/// tuple.
pub trait Exported {
    fn mark(scope: Scope);

    /// The first listed reducer this scope never claimed, if there is one.
    ///
    /// `Installs` closed the drift between what a segment says it installs and
    /// what it built, by making the list the body's return value. `Exports` is
    /// a type the feature names rather than a value it returns, so the same
    /// drift is closed from the other end: the list is checked against what
    /// was actually claimed, the moment the feature finishes installing.
    ///
    /// Without it, exporting something the feature never claimed type-checks,
    /// and a page below reads the reducer's `Default` forever - the state is
    /// created on first read, so nothing ever fails. Silence is the whole
    /// problem: a wrong export looks exactly like a feature that has not
    /// pushed an update yet.
    fn unclaimed(scope: Scope) -> Option<&'static str>;

    /// The listed reducers, for a check that reads a route tree without
    /// installing it.
    fn named(into: &mut Vec<Named>);
}

/// A type as a check reads it: what tells it apart, and what to call it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Named {
    pub id: std::any::TypeId,
    pub name: &'static str,
}

impl Named {
    pub fn of<T: 'static>() -> Self {
        Self {
            id: std::any::TypeId::of::<T>(),
            name: std::any::type_name::<T>(),
        }
    }
}

impl Exported for () {
    fn mark(_scope: Scope) {}

    fn unclaimed(_scope: Scope) -> Option<&'static str> {
        None
    }

    fn named(_into: &mut Vec<Named>) {}
}

fn missing<R: Reducer>(scope: Scope) -> Option<&'static str> {
    (!scope.claims::<R>()).then(|| std::any::type_name::<R>())
}

macro_rules! exported {
    ($($reducer:ident),+) => {
        impl<$($reducer: Reducer),+> Exported for ($($reducer,)+) {
            fn mark(scope: Scope) {
                $(scope.note_export::<$reducer>();)+
            }

            fn unclaimed(scope: Scope) -> Option<&'static str> {
                None$(.or_else(|| missing::<$reducer>(scope)))+
            }

            fn named(into: &mut Vec<Named>) {
                $(into.push(Named::of::<$reducer>());)+
            }
        }
    };
}

exported!(A);
exported!(A, B);
exported!(A, B, C);
exported!(A, B, C, D);
exported!(A, B, C, D, E);
exported!(A, B, C, D, E, F);
exported!(A, B, C, D, E, F, G);
exported!(A, B, C, D, E, F, G, H);
exported!(A, B, C, D, E, F, G, H, I);
exported!(A, B, C, D, E, F, G, H, I, J);
exported!(A, B, C, D, E, F, G, H, I, J, K);
exported!(A, B, C, D, E, F, G, H, I, J, K, L);
