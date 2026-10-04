//! A value for a route's field, where its type has a default - for
//! `RouteChain::samples`, which `routes!` writes.
//!
//! Which of the two `make`s applies is decided by method resolution on the
//! concrete field type: the one taking `Probe<T>` by reference wins where
//! `T: Default` holds, and the one on `&Probe<T>` is found one autoref later.

use std::marker::PhantomData;

#[doc(hidden)]
pub struct Probe<T>(PhantomData<fn() -> T>);

impl<T> Probe<T> {
    #[doc(hidden)]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

#[doc(hidden)]
pub trait ByDefault<T> {
    fn make(&self) -> Option<T>;
}

impl<T: Default> ByDefault<T> for Probe<T> {
    fn make(&self) -> Option<T> {
        Some(T::default())
    }
}

#[doc(hidden)]
pub trait Without<T> {
    fn make(&self) -> Option<T>;
}

impl<T> Without<T> for &Probe<T> {
    fn make(&self) -> Option<T> {
        None
    }
}
