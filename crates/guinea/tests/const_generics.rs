//! `#[reducer]` and `#[installs]` on functions with a const parameter beside
//! a type parameter.

use guinea::app::Harness;
use guinea::prelude::*;

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Recent<T, const N: usize> {
    pub items: Vec<T>,
}

#[reducer]
fn recent<T: Clone + PartialEq + std::fmt::Debug + Default + 'static, const N: usize>(
    this: &mut Recent<T, N>,
    item: T,
) {
    this.items.push(item);
    if this.items.len() > N {
        this.items.remove(0);
    }
}

feature! {
    pub Remembering<T: Clone + PartialEq + std::fmt::Debug + Default + 'static, const N: usize> {
        exports { Recent<T, N> }
    }
}

#[installs]
fn remembering<T: Clone + PartialEq + std::fmt::Debug + Default + 'static, const N: usize>(
    cx: &FeatureInitContext,
) -> anyhow::Result<Remembering<T, N>> {
    Ok(Remembering(cx.state::<Recent<T, N>>().plain()))
}

#[test]
fn a_reducer_over_a_const_generic_state_keeps_the_last_n() {
    let mut recent = Recent::<u8, 2>::default();

    recent.reduce(1);
    recent.reduce(2);
    recent.reduce(3);

    assert_eq!(recent.items, [2, 3]);
}

#[test]
fn a_feature_with_a_const_parameter_installs_its_state() {
    let h = Harness::new(0);

    h.install::<Remembering<u8, 2>>(&()).unwrap();

    assert!(h.state::<Recent<u8, 2>>().items.is_empty());
}
