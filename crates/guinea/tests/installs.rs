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

    #[derive(Clone)]
    pub struct Ready;

    feature! {
        pub Counting {
            exports { Count }
        }
    }

    #[installs]
    fn counting(cx: &Install,_ready: Ready) -> anyhow::Result<Counting> {
        let count = cx.state::<Count>().seed(Count(7)).plain();
        Ok(Counting(count))
    }

    feature! {
        pub Halfway {
            exports { Count }
        }
    }

    #[installs]
    fn halfway(cx: &Install,ready: Option<Ready>) -> anyhow::Result<Halfway> {
        let count = cx.state::<Count>().plain();
        ready.ok_or_else(|| anyhow::anyhow!("nothing provides Ready yet"))?;
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

    let listed = h.segment().scope().features();

    assert!(listed.is_empty(), "devtools list {listed:?}");
}

#[guinea::test(iterations = 1)]
fn what_a_feature_claimed_before_its_install_failed_is_not_claimed(h: &mut Harness) {
    assert!(h.install::<counting::Halfway>(&()).is_err(), "nothing provides Ready yet");

    assert!(!h.segment().scope().claims::<counting::Count>());
}

mod boasting {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Own(pub u32);

    #[reducer]
    fn own(this: &mut Own, to: u32) {
        this.0 = to;
    }

    pub struct Boasting;

    impl Plugin for Boasting {
        type Params = ();
        type Exports = (counting::Count,);

        fn build(app: &mut PluginBuilder, (): ()) -> anyhow::Result<Self> {
            app.state::<Own>().plain();
            Ok(Boasting)
        }
    }
}

#[guinea::test(iterations = 1)]
fn a_plugin_exporting_what_it_never_claimed_fails_and_names_the_reducer(h: &mut Harness) {
    let installed = catch_unwind(AssertUnwindSafe(|| h.plugin::<boasting::Boasting>(()).map(|_| ())))
        .map_err(said)
        .map(|installed| installed.map_err(|error| format!("{error:#}")));

    assert!(
        matches!(&installed, Ok(Err(refused)) if refused.contains("Count")),
        "{installed:?}"
    );
}

#[guinea::test(iterations = 1)]
fn a_plugin_refused_for_what_it_did_not_claim_keeps_nothing_it_did(h: &mut Harness) {
    let _ = catch_unwind(AssertUnwindSafe(|| h.plugin::<boasting::Boasting>(()).map(|_| ())));

    assert!(!h.application_scope().claims::<boasting::Own>());
}

thread_local! {
    static RUNS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

mod once {
    use super::*;

    feature! {
        pub Once {}
    }

    #[installs]
    fn once(_cx: &Install) -> anyhow::Result<Once> {
        RUNS.set(RUNS.get() + 1);
        Ok(Once)
    }
}

#[guinea::test(iterations = 1)]
fn a_feature_installed_twice_in_one_scope_is_refused_before_its_body_runs(h: &mut Harness) {
    h.install::<once::Once>(&()).unwrap();

    let again = catch_unwind(AssertUnwindSafe(|| h.install::<once::Once>(&()).is_ok())).map_err(said);

    assert_eq!(RUNS.get(), 1, "the body ran again: {again:?}");
    assert!(
        matches!(&again, Err(refused) if refused.contains("Once")),
        "{again:?}"
    );
}
