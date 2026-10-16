//! A guard on leaving reads the state it names where a page reads it, at the
//! moment of leaving - and declaring one claims or creates nothing.

use guinea::app::Harness;
use guinea::guard::Verdict;
use guinea::prelude::*;

mod drafting {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Draft {
        pub text: &'static str,
        pub dirty: bool,
    }

    #[reducer]
    fn draft(this: &mut Draft, text: &'static str) {
        this.text = text;
        this.dirty = true;
    }

    feature! {
        pub Drafting {
            exports { Draft }
        }
    }

    #[installs]
    fn drafting(cx: &FeatureInitContext) -> anyhow::Result<Drafting> {
        cx.block_leave_while::<Draft>(|draft| draft.dirty);
        let draft = cx
            .state::<Draft>()
            .seed(Draft {
                text: "saved on disk",
                dirty: false,
            })
            .plain();
        Ok(Drafting(draft))
    }

    feature! {
        pub Editing {}
    }

    #[installs]
    fn editing(cx: &FeatureInitContext) -> anyhow::Result<Editing> {
        cx.block_leave_while::<Draft>(|draft| draft.dirty);
        Ok(Editing)
    }

    feature! {
        pub Asking {}
    }

    #[installs]
    fn asking(cx: &FeatureInitContext) -> anyhow::Result<Asking> {
        cx.confirm_leave::<Draft>(
            |draft| draft.dirty,
            |_| guinea::guard::Ask::new("Leave the draft?", "Leave", "Stay"),
        );
        Ok(Asking)
    }
}

fn blocks(segment: &guinea::app::Segment<'_>) -> Vec<bool> {
    segment
        .scope()
        .leave_guards()
        .iter()
        .map(|guard| !matches!(guard(), Verdict::Allow))
        .collect()
}

#[guinea::test(iterations = 1)]
fn a_seed_holds_when_a_leave_guard_was_declared_first(h: &mut Harness) {
    h.install::<drafting::Drafting>(&()).unwrap();

    assert_eq!(h.state::<drafting::Draft>().text, "saved on disk");
}

#[guinea::test(iterations = 1)]
fn a_page_s_leave_guard_reads_the_draft_the_layout_above_exports(h: &mut Harness) {
    h.install::<drafting::Drafting>(&()).unwrap();
    let page = h.child();
    page.install::<drafting::Editing>(&()).unwrap();

    h.segment()
        .scope()
        .owner_of::<drafting::Draft>()
        .unwrap()
        .push::<drafting::Draft>("typed");
    assert!(page.state::<drafting::Draft>().dirty, "the page reads the draft as dirty");

    assert_eq!(blocks(&page), [true], "the page's guard let a dirty draft go");
}

#[guinea::test(iterations = 1)]
fn a_page_s_question_on_leaving_reads_the_draft_the_layout_above_exports(h: &mut Harness) {
    h.install::<drafting::Drafting>(&()).unwrap();
    let page = h.child();
    page.install::<drafting::Asking>(&()).unwrap();

    h.segment()
        .scope()
        .owner_of::<drafting::Draft>()
        .unwrap()
        .push::<drafting::Draft>("typed");

    assert_eq!(blocks(&page), [true], "the page's question let a dirty draft go unasked");
}

#[guinea::test(iterations = 1)]
fn a_leave_guard_on_state_nothing_in_reach_holds_lets_the_page_go(h: &mut Harness) {
    let page = h.child();
    page.install::<drafting::Editing>(&()).unwrap();

    assert_eq!(blocks(&page), [false]);
    assert!(page.scope().owner_of::<drafting::Draft>().is_none());
}
