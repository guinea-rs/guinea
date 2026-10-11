//! The same shape of feature, with no actor in it - and nothing outside can
//! tell.
//!
//! This exists to hold the boundary honest. An actor is one way for a domain
//! to run its logic; the framework must not make it the only way, and the way
//! to check that is to write a feature without one. Here the state is an
//! ordinary `Rc<RefCell<_>>` on the UI thread, actions are answered by a
//! closure, and the domain schedules its own work with a timer - no mailbox,
//! no `Handler`, no `Addr`.
//!
//! What the UI sees is identical to `processes`: a reducer to read and
//! `dispatch.emit(Refresh)` to act. It named no actor before and there is none
//! to name now, which is the whole claim.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use guinea::prelude::*;

use super::contracts::{self, Listed, Refresh};

/// Everything this feature is: some state of its own, and the way back into
/// the reducer.
struct Catalogue {
    scans: u32,
    push: Push<contracts::Services>,
}

impl Catalogue {
    fn scan(&mut self) {
        self.scans += 1;
        self.push.send(Listed::Items(vec![
            "sshd.service".to_string(),
            "docker.service".to_string(),
            format!("cron.service (scan {})", self.scans),
        ]));
    }
}

feature! {
    pub ServicesFeature {
        exports { contracts::Services }
    }
}

#[installs]
fn services(cx: &FeatureInitContext) -> anyhow::Result<ServicesFeature> {
    let services = cx.state::<contracts::Services>().plain();

    // The domain's own state, kept alive by what answers and what ticks -
    // both belong to the scope - rather than by a mailbox. `RefCell` and not
    // a lock: this all happens on the one thread that draws, which is the
    // case the actor model is *an* answer to rather than the answer.
    let catalogue = Rc::new(RefCell::new(Catalogue {
        scans: 0,
        push: services.pusher(),
    }));

    // Answering an action is a closure. `dispatch.emit(Refresh)` from any
    // page reaches it exactly as it reaches an actor, because the scope is
    // keyed by the action and never by whoever answers it.
    let answering = catalogue.clone();
    cx.answers::<Refresh>(move |_| answering.borrow_mut().scan());

    // And the domain runs on its own schedule, which is the part an actor
    // is usually reached for. The timer belongs to the scope, so it stops
    // when the page does.
    cx.repeat(Duration::from_secs(5), move || catalogue.borrow_mut().scan());

    services.emit(Refresh);
    Ok(ServicesFeature(services))
}
