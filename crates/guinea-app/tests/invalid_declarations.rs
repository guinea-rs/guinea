use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea_app::app::{FeatureBuilder, start};
use guinea_macros::{feature, installs};

#[derive(Clone)]
struct Missing;

feature! { BrokenApplication {} }

thread_local! {
    static RAN: Cell<bool> = const { Cell::new(false) };
}

#[installs]
fn broken_application(
    _app: &mut FeatureBuilder,
    _missing: Missing,
) -> anyhow::Result<BrokenApplication> {
    RAN.set(true);
    Ok(BrokenApplication)
}

#[test]
fn startup_validates_inventory_before_entering_application_code() {
    let panic = catch_unwind(AssertUnwindSafe(|| {
        start("test", |app| {
            RAN.set(true);
            app.install::<BrokenApplication>()
        })
    }))
    .err()
    .expect("invalid declarations must panic");
    let message = panic.downcast_ref::<String>().unwrap();
    insta::assert_snapshot!(message.replace('\\', "/"));
    assert!(!RAN.get());
}
