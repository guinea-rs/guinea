//! What a source does while it makes its next item is traced under a pull of
//! that source, not left without a cause: a stream that publishes from inside
//! its poll is otherwise a publish nothing observed started.
//!
//! A process of its own, because the dispatcher is process-wide.

use futures_core::Stream;
use guinea_core::actor::{
    Addr, Cx, Handler, UiDispatcher, UiTask, UiThreadToken, set_ui_dispatcher,
};
use guinea_core::trace::{self, Cause, Point, Trace};
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

struct Making {
    left: u32,
}

impl Stream for Making {
    type Item = u32;

    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<u32>> {
        if self.left == 0 {
            return Poll::Ready(None);
        }
        self.left -= 1;

        trace::mark(|| Point::Note("making an item".into()));

        Poll::Ready(Some(self.left))
    }
}

struct Open(Making);

struct Made;

#[derive(Debug)]
struct Maker;

guinea_macros::actor! {
    Maker {
        handlers {
            Open => { bg Made }
            Made
        }
    }
}

impl Handler<Open> for Maker {
    fn handle(&mut self, Open(source): Open, cx: Cx<Self, Open>) {
        cx.spawn_source(source, |_| Made);
    }
}

impl Handler<Made> for Maker {
    fn handle(&mut self, _: Made, _cx: Cx<Self, Made>) {}
}

struct Nowhere;

impl UiDispatcher for Nowhere {
    fn init(&self) {}

    fn dispatch(&self, _: UiTask) {}
}

#[derive(Clone, Debug)]
struct Heard {
    id: Cause,
    parent: Option<Cause>,
    point: Point,
}

#[test]
fn what_a_source_does_for_its_next_item_is_under_a_pull_of_it() {
    set_ui_dispatcher(Nowhere);

    let heard = Rc::new(RefCell::new(Vec::<Heard>::new()));
    let hearing = heard.clone();
    trace::observe(move |record| {
        if let Trace::Begin(record) | Trace::Mark(record) = record {
            hearing.borrow_mut().push(Heard {
                id: record.id,
                parent: record.parent,
                point: record.point.clone(),
            });
        }
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let addr = Addr::new_managed(Maker, UiThreadToken::dangerously_create_token_unchecked());
        addr.send(Open(Making { left: 2 }));

        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    });
    trace::stop_observing();

    let heard = heard.borrow();
    let at = |id: Cause| heard.iter().find(|one| one.id == id);
    let source = heard
        .iter()
        .find(|one| matches!(one.point, Point::Source { .. }))
        .expect("the source was opened");
    let making = heard
        .iter()
        .filter(|one| matches!(&one.point, Point::Note(text) if text == "making an item"))
        .collect::<Vec<_>>();

    assert_eq!(making.len(), 2, "the source made two items:\n{heard:#?}");
    for item in making {
        let pull = item.parent.and_then(at);
        assert!(
            matches!(
                pull,
                Some(Heard {
                    parent: None,
                    point: Point::Pull { source: pulled, .. },
                    ..
                }) if *pulled == source.id.get()
            ),
            "making an item is under a pull of its source, which is a root:\n{heard:#?}"
        );
    }
}
