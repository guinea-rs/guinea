//! What a segment may read, decided at build time.
//!
//! Both halves of the answer are already written down: `routes!` knows who
//! sits above whom, and the author declares what each segment `Installs` and
//! what each feature `Exports`. Joining them turns "no scope owns this" from a
//! panic on the first render into an error at the read itself.
//!
//! It lives here rather than in `guinea-core` because [`Feature`] does, and a
//! blanket impl has to be in the crate that owns the trait.

use std::marker::PhantomData;
use std::rc::Rc;

use guinea_core::feature::{Bound, Dispatch};
use guinea_core::scope::Reducer;

use super::traits::Feature;
use crate::app::{AppFeature, Plugin};

/// Where a segment sits in its route tree, written by `routes!`.
///
/// A page belongs to one tree: its ancestry is part of the type, so the same
/// page in two trees would need two answers and gets a conflicting-impl error
/// instead of a wrong one.
pub trait Segment: 'static {
    /// `<Self as Page>::Installs` - the macro names it because it knows the
    /// backend and the trait; the author declares it on the impl.
    type Installs;
    /// The segments above, innermost first, as a cons list: `(Tabs, (Shell, ()))`.
    type Above;
}

/// Which of several impls applied - a disambiguator, not information.
///
/// Coherence is the whole reason it exists: "the head matches" and "something
/// in the tail matches" are two impls that rustc cannot see are exclusive.
/// Carrying the position in the type keeps them apart.
///
/// It leaks one character into the call site - `cx.read::<R, _>()` - because
/// Rust has no partial turbofish.
pub struct Here;
pub struct There<I>(PhantomData<I>);

/// The element of a tuple that matched, at the position `I` counts to.
pub struct At<I>(PhantomData<I>);

/// Membership in a feature's `Exports`.
pub trait Lists<R, I> {}

/// What a segment's `Installs` publishes: one feature, a reducer it claimed
/// directly, or a tuple of either.
///
/// The index's first step says which of those shapes matched, which keeps the
/// impls apart even though nothing stops a tuple implementing [`Feature`].
pub trait Provides<R, I> {}

impl<F: Feature, R, I> Provides<R, (Here, I)> for F where F::Exports: Lists<R, I> {}

/// A reducer the segment claimed itself, without a feature between.
///
/// `cx.state::<R>()` already hands back a [`Bound<R>`], so declaring it costs
/// a segment nothing it was not already holding - and a claim that goes
/// undeclared is exactly a claim nothing below can see, which is what the
/// declaration is for.
impl<R: Reducer> Provides<R, (There<Here>, Here)> for Bound<R> {}

impl<F: AppFeature, R, I> Provides<R, (There<There<Here>>, I)> for F where F::Exports: Lists<R, I> {}

impl<P: Plugin, R, I> Provides<R, (There<There<There<Here>>>, I)> for P where P::Exports: Lists<R, I> {}

/// The application as a segment: one feature or plugin it installs, at the
/// top of every chain, for the pages below to read what it exports.
///
/// `routes!` puts one above every segment for each line of its `app { .. }`
/// block; `Application<()>` stands for a line its `#[cfg(..)]` turned off. A
/// chain written by hand lists them itself, outermost last:
/// `type Above = (Shell, (Application<Localisation>, ()))`.
pub struct Application<T>(PhantomData<T>);

impl<T: 'static> Segment for Application<T> {
    type Installs = T;
    type Above = ();
}

type P0 = Here;
type P1 = There<P0>;
type P2 = There<P1>;
type P3 = There<P2>;
type P4 = There<P3>;
type P5 = There<P4>;
type P6 = There<P5>;
type P7 = There<P6>;
type P8 = There<P7>;
type P9 = There<P8>;
type P10 = There<P9>;
type P11 = There<P10>;

macro_rules! tuple {
    ($all:tt; $($element:ident $position:ty),+) => {
        $(tuple!(@element $all; $element $position);)+
    };
    (@element ($($all:ident),+); $element:ident $position:ty) => {
        impl<$($all,)+> Lists<$element, $position> for ($($all,)+) {}

        impl<$($all,)+ R, I> Provides<R, (At<$position>, I)> for ($($all,)+)
        where
            $element: Provides<R, I>
        {
        }
    };
}

tuple!((A); A P0);
tuple!((A, B); A P0, B P1);
tuple!((A, B, C); A P0, B P1, C P2);
tuple!((A, B, C, D); A P0, B P1, C P2, D P3);
tuple!((A, B, C, D, E); A P0, B P1, C P2, D P3, E P4);
tuple!((A, B, C, D, E, F); A P0, B P1, C P2, D P3, E P4, F P5);
tuple!((A, B, C, D, E, F, G); A P0, B P1, C P2, D P3, E P4, F P5, G P6);
tuple!((A, B, C, D, E, F, G, H); A P0, B P1, C P2, D P3, E P4, F P5, G P6, H P7);
tuple!((A, B, C, D, E, F, G, H, J); A P0, B P1, C P2, D P3, E P4, F P5, G P6, H P7, J P8);
tuple!(
    (A, B, C, D, E, F, G, H, J, K);
    A P0, B P1, C P2, D P3, E P4, F P5, G P6, H P7, J P8, K P9
);
tuple!(
    (A, B, C, D, E, F, G, H, J, K, L);
    A P0, B P1, C P2, D P3, E P4, F P5, G P6, H P7, J P8, K P9, L P10
);
tuple!(
    (A, B, C, D, E, F, G, H, J, K, L, M);
    A P0, B P1, C P2, D P3, E P4, F P5, G P6, H P7, J P8, K P9, L P10, M P11
);

/// Proof that a segment may read `R`: it installed the feature that exports
/// it, or a segment above it did - the application's own segment included.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot read `{R}` from here",
    label = "no feature in reach exports it",
    note = "a segment reads what it installed itself and what a segment above it listed in `Exports` - for what the application exports, list its feature or plugin in the `app {{ .. }}` block of `routes!`"
)]
pub trait Reaches<R, I> {}

impl<S: Segment, R, I> Reaches<R, (Here, I)> for S where S::Installs: Provides<R, I> {}
impl<S: Segment, R, I> Reaches<R, (There<Here>, I)> for S where S::Above: Reaches<R, I> {}

/// A page or a layout being drawn, as far as reading goes - whatever backend
/// draws it.
///
/// What lets a plugin offer one shortcut on every backend, the way
/// [`Services`](crate::services::Services) lets it offer one on every context
/// that holds services. A segment reads `R` through it and is drawn again when
/// `R` changes: by subscribing, or by drawing every frame anyway. A backend
/// that builds its view once and binds it is not one of these - it binds.
pub trait Reads {
    /// The segment being drawn, whose reach decides what it may read.
    type Segment: Segment;

    /// `R` as it is now, and what may be asked of the feature that owns it.
    fn read<R, I>(&mut self) -> (Rc<R>, Dispatch)
    where
        R: Reducer + PartialEq,
        Self::Segment: Reaches<R, I>;

    /// What may be asked of the feature that owns `R`, without reading it:
    /// the segment is not drawn again when `R` changes.
    fn dispatch<R, I>(&self) -> Dispatch
    where
        R: Reducer,
        Self::Segment: Reaches<R, I>;
}

// The ancestors are a cons list rather than a segment, so they walk their own
// way.
impl<H: Segment, T, R, I> Reaches<R, (Here, I)> for (H, T) where H::Installs: Provides<R, I> {}
impl<H, T, R, I> Reaches<R, (There<Here>, I)> for (H, T) where T: Reaches<R, I> {}
