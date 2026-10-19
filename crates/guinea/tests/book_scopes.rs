//! The examples on the book's page about scopes, run. `cargo xtask docs`
//! copies what is between the marks into it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use guinea::app::Harness;
use guinea::prelude::*;

mod inbox {
    use super::*;

    //@show scopes: a feature the layout installs
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Unread(pub u32);

    #[reducer]
    fn unread(this: &mut Unread, arrived: u32) {
        this.0 += arrived;
    }

    pub struct Arrived;

    #[derive(Debug)]
    pub struct Mailbox {
        push: Push<Unread>,
    }

    actor! {
        Mailbox {
            handlers { Arrived }
        }
    }

    #[handler]
    fn arrived(In(this, Arrived, _): In<Mailbox, Arrived>) {
        this.push.send(1);
    }

    feature! {
        pub Inbox {
            exports { Unread }
        }
    }

    #[installs]
    fn inbox(cx: &Install) -> anyhow::Result<Inbox> {
        let (unread, _) = cx.state::<Unread>().driven_by(|push| Mailbox { push });
        Ok(Inbox(unread))
    }
    //@show-end
}

mod composing {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Draft(pub String);

    #[reducer]
    fn draft(this: &mut Draft, text: String) {
        this.0 = text;
    }

    feature! {
        pub Composing {
            exports { Draft }
        }
    }

    //@show scopes: a feature the page installs
    #[installs]
    fn composing(cx: &Install<Rc<Cell<u32>>>) -> anyhow::Result<Composing> {
        let draft = cx.state::<Draft>().plain();

        let saved = cx.params().clone();
        cx.repeat(Duration::from_secs(1), move || saved.set(saved.get() + 1));

        Ok(Composing(draft))
    }
    //@show-end
}

use composing::{Composing, Draft};
use inbox::{Arrived, Inbox, Unread};

//@show scopes: a page reads what the layout exports
#[guinea::test]
fn a_page_reads_what_the_layout_above_exports(h: &mut Harness) {
    h.install::<Inbox>(&()).unwrap();
    let page = h.child();

    page.dispatch::<Unread>().emit(Arrived);
    h.settled();

    assert_eq!(page.state::<Unread>().0, 1);
    assert_eq!(h.state::<Unread>().0, 1);
}
//@show-end

//@show scopes: leaving a page ends what it started
#[guinea::test]
fn leaving_a_page_ends_what_it_started(h: &mut Harness) {
    h.install::<Inbox>(&()).unwrap();
    let saved = Rc::new(Cell::new(0));

    let page = h.child();
    page.install::<Composing>(&saved).unwrap();
    h.advance(Duration::from_secs(3));
    assert_eq!(saved.get(), 3);

    page.leave();
    h.advance(Duration::from_secs(3));
    assert_eq!(saved.get(), 3);

    h.dispatch::<Unread>().emit(Arrived);
    h.settled();
    assert_eq!(h.state::<Unread>().0, 1);
}
//@show-end

//@show scopes: what a page claims is not seen above it
#[guinea::test]
fn what_a_page_claims_is_not_seen_above_it(h: &mut Harness) {
    let page = h.child();
    page.install::<Composing>(&Rc::default()).unwrap();

    assert_eq!(page.scope().owner_of::<Draft>(), Some(page.scope()));
    assert_eq!(h.segment().scope().owner_of::<Draft>(), None);
}
//@show-end
