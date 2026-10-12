use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Inspect, Inspector};

macro_rules! values {
    ($($ty:ty),* $(,)?) => {$(
        impl Inspect for $ty {
            fn inspect(&self, at: Inspector<'_>) {
                at.value(&self);
            }
        }
    )*};
}

values!(
    bool, char, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, str,
    String, Path, PathBuf, Duration, Instant, (),
);

impl<T: Inspect + ?Sized> Inspect for &T {
    fn inspect(&self, at: Inspector<'_>) {
        (**self).inspect(at);
    }
}

impl<T: Inspect + ?Sized> Inspect for Box<T> {
    fn inspect(&self, at: Inspector<'_>) {
        (**self).inspect(at);
    }
}

impl<T: Inspect + ?Sized> Inspect for Rc<T> {
    fn inspect(&self, at: Inspector<'_>) {
        (**self).inspect(at);
    }
}

impl<T: Inspect + ?Sized> Inspect for Arc<T> {
    fn inspect(&self, at: Inspector<'_>) {
        (**self).inspect(at);
    }
}

impl<T: Inspect + Copy> Inspect for Cell<T> {
    fn inspect(&self, at: Inspector<'_>) {
        self.get().inspect(at);
    }
}

impl<T: Inspect + ?Sized> Inspect for RefCell<T> {
    fn inspect(&self, at: Inspector<'_>) {
        match self.try_borrow() {
            Ok(value) => value.inspect(at),
            Err(_) => at.opaque("<borrowed>"),
        }
    }
}

impl<T: Inspect> Inspect for Option<T> {
    fn inspect(&self, at: Inspector<'_>) {
        match self {
            Some(value) => at.tuple("Some").field("0", value).finish(),
            None => at.unit("None"),
        }
    }
}

impl<T: Inspect, E: Inspect> Inspect for Result<T, E> {
    fn inspect(&self, at: Inspector<'_>) {
        match self {
            Ok(value) => at.tuple("Ok").field("0", value).finish(),
            Err(error) => at.tuple("Err").field("0", error).finish(),
        }
    }
}

impl<T: Inspect> Inspect for [T] {
    fn inspect(&self, at: Inspector<'_>) {
        at.list(self.len(), self.iter());
    }
}

impl<T: Inspect, const N: usize> Inspect for [T; N] {
    fn inspect(&self, at: Inspector<'_>) {
        at.list(N, self.iter());
    }
}

impl<T: Inspect> Inspect for Vec<T> {
    fn inspect(&self, at: Inspector<'_>) {
        at.list(self.len(), self.iter());
    }
}

impl<T: Inspect> Inspect for VecDeque<T> {
    fn inspect(&self, at: Inspector<'_>) {
        at.list(self.len(), self.iter());
    }
}

impl<T: Inspect, S> Inspect for HashSet<T, S> {
    fn inspect(&self, at: Inspector<'_>) {
        at.list(self.len(), self.iter());
    }
}

impl<T: Inspect> Inspect for BTreeSet<T> {
    fn inspect(&self, at: Inspector<'_>) {
        at.list(self.len(), self.iter());
    }
}

impl<K: Inspect, V: Inspect, S> Inspect for HashMap<K, V, S> {
    fn inspect(&self, at: Inspector<'_>) {
        at.map(self.len(), self.iter());
    }
}

impl<K: Inspect, V: Inspect> Inspect for BTreeMap<K, V> {
    fn inspect(&self, at: Inspector<'_>) {
        at.map(self.len(), self.iter());
    }
}

macro_rules! tuples {
    ($(($($name:ident $index:tt),+))*) => {$(
        impl<$($name: Inspect),+> Inspect for ($($name,)+) {
            fn inspect(&self, at: Inspector<'_>) {
                at.tuple("")
                    $(.field(stringify!($index), &self.$index))+
                    .finish();
            }
        }
    )*};
}

tuples! {
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
}
