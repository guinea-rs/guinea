//! A guard's question, put to the user by the window and answered back.
//!
//! The window is the application's own `.slint`, so guinea has nowhere to draw
//! a dialog into. It says when there is a question and when it is gone, and
//! takes the answer; the window draws it.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use guinea_core::guard::Ask;
use guinea_router::router::Router;

use crate::Slint;

type Asker = Rc<dyn Fn(Option<Ask>)>;

thread_local! {
    static ASKER: RefCell<Option<Asker>> = const { RefCell::new(None) };
    static ROUTER: RefCell<Option<Weak<Router<Slint>>>> = const { RefCell::new(None) };
}

/// Shows a guard's question: `ask` is called with it when a guard asks one,
/// and with `None` once it is gone - answered, or dropped for a later
/// navigation. What the window shows answers through [`answer`].
///
/// Called before [`run`](crate::run), with a weak handle to the window:
///
/// ```ignore
/// let window = AppWindow::new()?;
/// let shown = window.as_weak();
/// guinea_slint::on_question(move |ask| {
///     let Some(window) = shown.upgrade() else { return };
///     window.set_question(ask.map(|ask| ask.text).unwrap_or_default().into());
/// });
/// window.on_answered(|allowed| guinea_slint::answer(allowed));
/// ```
///
/// Without it a question has nowhere to go, and the navigation that asked
/// waits for an answer that never comes.
pub fn on_question(ask: impl Fn(Option<Ask>) + 'static) {
    ASKER.with(|asker| *asker.borrow_mut() = Some(Rc::new(ask)));
}

/// Answers the question on screen: `true` goes on with the navigation that
/// asked, `false` stays. Does nothing when there is none.
pub fn answer(allowed: bool) {
    let router = ROUTER.with(|router| router.borrow().as_ref().and_then(Weak::upgrade));
    if let Some(router) = router {
        router.answer(allowed);
    }
}

/// Wires the router [`run`](crate::run) made to whatever shows its questions,
/// for as long as the returned handle lives.
pub(crate) fn install(router: &Rc<Router<Slint>>) -> guinea_router::router::RouteHookHandle {
    ROUTER.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(router)));

    let asked = Rc::downgrade(router);
    router.on_question(move || {
        let Some(router) = asked.upgrade() else {
            return;
        };
        let pending = router.pending();

        match ASKER.with(|asker| asker.borrow().clone()) {
            Some(ask) => ask(pending),
            None if pending.is_some() => tracing::warn!(
                "a guard asked a question and nothing shows it; see guinea_slint::on_question"
            ),
            None => {}
        }
    })
}

pub(crate) fn clear() {
    ROUTER.with(|slot| *slot.borrow_mut() = None);
    ASKER.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use guinea_app::feature::FeatureInitContext;
    use guinea_core::actor::UiThreadToken;
    use guinea_core::guard::Verdict;
    use guinea_router::router::{Navigation, RouteChain, SegmentEntry};

    use super::*;
    use crate::{PageCx, page_chain};

    struct Draft;

    impl crate::Page for Draft {
        type Params = ();
        type Installs = ();

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            ctx.on_leave(|| Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep")));
            Ok(())
        }

        fn bind(_cx: PageCx<Self>) {}
    }

    struct Elsewhere;

    impl crate::Page for Elsewhere {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn bind(_cx: PageCx<Self>) {}
    }

    #[derive(Clone, PartialEq, Debug)]
    struct ToElsewhere;

    impl RouteChain<Slint> for ToElsewhere {
        fn chain(&self) -> &'static [SegmentEntry<Slint>] {
            page_chain::<Elsewhere>()
        }

        fn params(&self) -> Vec<Box<dyn Any>> {
            vec![Box::new(())]
        }

        fn name(&self) -> &'static str {
            "Elsewhere"
        }
    }

    #[test]
    fn a_question_reaches_the_window_and_its_answer_the_router() {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let router = Rc::new(Router::<Slint>::new(token));
        let _question = install(&router);

        let asked = Rc::new(RefCell::new(Vec::new()));
        let seen = asked.clone();
        on_question(move |ask| seen.borrow_mut().push(ask.map(|ask| ask.text)));

        router
            .activate(page_chain::<Draft>(), vec![Box::new(())])
            .expect("the draft installs");

        let outcome = router.navigate(ToElsewhere).expect("the navigation is taken up");
        assert!(matches!(outcome, Navigation::Deferred));
        assert_eq!(*asked.borrow(), [Some("Discard the draft?".to_string())]);

        answer(true);
        assert_eq!(*asked.borrow(), [Some("Discard the draft?".to_string()), None]);
        assert!(router.pending().is_none());
        assert_eq!(
            router.current_route::<ToElsewhere>(),
            Some(ToElsewhere),
            "the answer let the navigation through"
        );

        clear();
    }
}
