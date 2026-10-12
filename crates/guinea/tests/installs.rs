//! What installing a feature leaves behind: in the scope, for the pages
//! below, and for devtools - when the install worked, and when it did not.

use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::prelude::*;

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .unwrap_or_default()
}

mod counting {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Count(pub u32);

    #[reducer]
    fn count(this: &mut Count, by: u32) {
        this.0 += by;
    }

    pub struct Ready;

    feature! {
        pub Counting {
            exports { Count }
        }
    }

    #[installs]
    fn counting(cx: &FeatureInitContext) -> anyhow::Result<Counting> {
        cx.require::<Ready>()?;
        let count = cx.state::<Count>().seed(Count(7)).plain();
        Ok(Counting(count))
    }

    feature! {
        pub Halfway {
            exports { Count }
        }
    }

    #[installs]
    fn halfway(cx: &FeatureInitContext) -> anyhow::Result<Halfway> {
        let count = cx.state::<Count>().plain();
        cx.require::<Ready>()?;
        Ok(Halfway(count))
    }
}

#[guinea::test(iterations = 1)]
fn a_feature_whose_install_failed_can_be_installed_once_what_it_needs_is_there(h: &mut Harness) {
    assert!(h.install::<counting::Counting>(&()).is_err(), "nothing provides Ready yet");
    h.provide(counting::Ready);

    let again = catch_unwind(AssertUnwindSafe(|| h.install::<counting::Counting>(&()).is_ok()))
        .map_err(said);

    assert_eq!(again, Ok(true), "the second install, after the first one failed");
}

#[guinea::test(iterations = 1)]
fn a_feature_whose_install_failed_exports_nothing_to_the_page_below(h: &mut Harness) {
    assert!(h.install::<counting::Counting>(&()).is_err(), "nothing provides Ready yet");

    let page = h.child();
    let read = page.context().read::<counting::Count>();

    assert!(read.is_none(), "the page below reads {read:?} from a feature that never installed");
}

#[guinea::test(iterations = 1)]
fn a_feature_whose_install_failed_is_not_listed_as_installed(h: &mut Harness) {
    assert!(h.install::<counting::Counting>(&()).is_err(), "nothing provides Ready yet");

    let listed = h.segment().context().scope.features();

    assert!(listed.is_empty(), "devtools list {listed:?}");
}

#[guinea::test(iterations = 1)]
fn what_a_listing_says_a_segment_s_own_reducer_exports_is_what_a_page_below_reads(h: &mut Harness) {
    let listed = guinea::feature::Listed::reducer::<counting::Count>();
    let listed_below = listed
        .exports
        .contains(&guinea::core::feature::Named::of::<counting::Count>());

    h.segment().context().state::<counting::Count>().plain();
    let read_below = h.child().context().read::<counting::Count>().is_some();

    assert_eq!(
        listed_below, read_below,
        "the listing says a page below reads it: {listed_below}; a page below reads it: {read_below}"
    );
}

#[guinea::test(iterations = 1)]
fn what_a_feature_claimed_before_its_install_failed_is_not_claimed(h: &mut Harness) {
    assert!(h.install::<counting::Halfway>(&()).is_err(), "nothing provides Ready yet");

    assert!(!h.segment().context().scope.claims::<counting::Count>());
}
