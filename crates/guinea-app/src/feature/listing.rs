//! What a segment says it installs, read without installing it.
//!
//! A route tree is checked as a whole before its first navigation, and the
//! check has only types to go on: each segment's `Installs`, and each
//! feature's `Exports`. This turns the first into a list the check can walk.

use guinea_core::feature::{Bound, Exported, Named};
use guinea_core::scope::Reducer;

use super::traits::Feature;

/// One thing a segment installs.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub what: Named,
    /// A feature, or a reducer the segment claimed itself.
    pub feature: bool,
    /// What segments below may read of it.
    pub exports: Vec<Named>,
}

impl Listed {
    pub fn feature<F: Feature>() -> Self {
        let mut exports = Vec::new();
        F::Exports::named(&mut exports);
        Self {
            what: Named::of::<F>(),
            feature: true,
            exports,
        }
    }

    pub fn reducer<R: Reducer>() -> Self {
        Self {
            what: Named::of::<R>(),
            feature: false,
            exports: vec![Named::of::<R>()],
        }
    }
}

/// A segment's `Installs`: one feature, a reducer it claimed directly, an
/// `Option` of either, or a tuple of those.
pub trait Lists: 'static {
    fn list(into: &mut Vec<Listed>);
}

impl<F: Feature> Lists for F {
    fn list(into: &mut Vec<Listed>) {
        into.push(Listed::feature::<F>());
    }
}

impl<R: Reducer> Lists for Bound<R> {
    fn list(into: &mut Vec<Listed>) {
        into.push(Listed::reducer::<R>());
    }
}

impl<T: Lists> Lists for Option<T> {
    fn list(into: &mut Vec<Listed>) {
        T::list(into);
    }
}

impl Lists for () {
    fn list(_into: &mut Vec<Listed>) {}
}

macro_rules! lists {
    ($($element:ident),+) => {
        impl<$($element: Lists),+> Lists for ($($element,)+) {
            fn list(into: &mut Vec<Listed>) {
                $($element::list(into);)+
            }
        }
    };
}

lists!(A);
lists!(A, B);
lists!(A, B, C);
lists!(A, B, C, D);
lists!(A, B, C, D, E);
lists!(A, B, C, D, E, F);
lists!(A, B, C, D, E, F, G);
lists!(A, B, C, D, E, F, G, H);
lists!(A, B, C, D, E, F, G, H, I);
lists!(A, B, C, D, E, F, G, H, I, J);
lists!(A, B, C, D, E, F, G, H, I, J, K);
lists!(A, B, C, D, E, F, G, H, I, J, K, L);
