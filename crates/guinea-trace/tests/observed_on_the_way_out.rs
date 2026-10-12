//! Alone in its binary: what a thread-local destructor sees depends on the
//! order the thread's locals were made in.

use std::cell::RefCell;
use std::panic::catch_unwind;
use std::sync::Mutex;

use guinea_trace::{is_observed, observe};

static ASKED: Mutex<Option<Result<bool, String>>> = Mutex::new(None);

struct AsksOnTheWayOut;

impl Drop for AsksOnTheWayOut {
    fn drop(&mut self) {
        let asked = catch_unwind(is_observed).map_err(|panic| {
            panic
                .downcast_ref::<&str>()
                .map(|text| text.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default()
        });

        *ASKED.lock().unwrap() = Some(asked);
    }
}

thread_local! {
    static OUTLIVES_THE_OBSERVER: RefCell<Option<AsksOnTheWayOut>> = const { RefCell::new(None) };
}

#[test]
fn a_local_that_outlives_the_observer_still_asks_whether_it_is_observed() {
    std::thread::spawn(|| {
        OUTLIVES_THE_OBSERVER.with(|local| *local.borrow_mut() = Some(AsksOnTheWayOut));
        observe(|_| {});
    })
    .join()
    .unwrap();

    let asked = ASKED.lock().unwrap().take();
    assert_eq!(asked, Some(Ok(false)), "asked after the thread's observer was gone");
}
