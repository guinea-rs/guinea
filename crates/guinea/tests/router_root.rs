#![cfg(all(windows, feature = "winui"))]

//! A window the way the application opens one: a navigator over the route,
//! the layout it roots at, the outlet that layout draws, and a guard's
//! question answered through the dialog the window puts up, the way a user
//! answers it.

use std::cell::{Cell, RefCell};

use guinea::app::Harness;
use guinea::enter::{Enter, EnterCx};
use guinea::feature::FeatureInitContext;
use guinea::winui::harness::Mounted;
use guinea::winui::*;
use guinea_core::feature::Bound;
use guinea_core::scope::Scope;
use guinea_macros::reducer;
use windows_reactor::{TextBlock, View};

thread_local! {
    static SHOWN: RefCell<Vec<String>> = RefCell::default();
    static INSTALLED: Cell<Option<Scope>> = Cell::default();
    static ENTERED: Cell<usize> = Cell::default();
}

#[derive(Default, Clone, PartialEq, Debug)]
struct Load {
    percent: u32,
}

#[reducer]
fn load(this: &mut Load, percent: u32) {
    this.percent = percent;
}

struct Counted;

impl Enter for Counted {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        ENTERED.with(|entered| entered.set(entered.get() + 1));
        Verdict::Allow
    }
}

#[derive(Clone, PartialEq, Debug, Route)]
enum Place {
    Process { pid: u32 },
    Editor,
    #[guard(Counted)]
    Guarded,
    Unreachable,
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Place>();
        cx.outlet(&nav, |place, cx| match place {
            Place::Process { pid } => cx.mount::<Process>(pid),
            Place::Editor => cx.mount::<Editor>(&()),
            Place::Guarded => cx.mount::<Guarded>(&()),
            Place::Unreachable => cx.mount::<Unreachable>(&()),
        })
    }
}

#[derive(Default)]
struct Process {
    pid: u32,
}

#[page]
impl Page for Process {
    type Params = u32;
    type Installs = Bound<Load>;

    fn install(ctx: &FeatureInitContext, pid: &u32) -> anyhow::Result<(Self, Bound<Load>)> {
        INSTALLED.set(Some(ctx.scope));
        let load = ctx.state::<Load>().seed(Load::default()).plain();
        Ok((Self { pid: *pid }, load))
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (load, _) = cx.read::<Load>();
        shown(format!("process {} at {}%", self.pid, load.percent))
    }
}

#[derive(Default)]
struct Editor;

#[page]
impl Page for Editor {
    fn leaving(&self) -> Verdict {
        Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep"))
    }

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        shown("editor".to_string())
    }
}

#[derive(Default)]
struct Guarded;

#[page]
impl Page for Guarded {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        shown("guarded".to_string())
    }
}

#[derive(Default)]
struct Unreachable;

#[page]
impl Page for Unreachable {
    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Self, ())> {
        anyhow::bail!("the agent is not reachable")
    }

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        shown("unreachable".to_string())
    }
}

fn shown(text: String) -> View {
    SHOWN.with(|shown| shown.borrow_mut().push(text.clone()));
    TextBlock::new().text(text).into()
}

fn last_shown() -> String {
    SHOWN.with(|shown| shown.borrow().last().cloned().unwrap_or_default())
}

#[guinea::test]
fn the_same_page_with_other_params_is_a_new_node_that_hears_its_own_scope(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Process { pid: 1 }).unwrap();
    assert_eq!(last_shown(), "process 1 at 0%");

    app.navigate(Place::Process { pid: 2 });
    assert_eq!(last_shown(), "process 2 at 0%");

    let installed = INSTALLED.get().unwrap();
    installed.push::<Load>(40);
    app.settle();
    assert_eq!(
        last_shown(),
        "process 2 at 40%",
        "the page listens to the scope it was installed into, not the one before"
    );
}

#[guinea::test]
fn a_page_that_minds_being_left_asks_and_the_answer_decides(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Editor).unwrap();
    assert_eq!(app.question(), None);

    app.navigate(Place::Guarded);
    assert_eq!(app.question().as_deref(), Some("Discard the draft?"), "the question is on screen");
    assert_eq!(last_shown(), "editor");

    app.answer(false);
    assert_eq!(app.question(), None);
    assert_eq!(app.route(), Place::Editor, "kept");

    app.navigate(Place::Guarded);
    assert!(app.question().is_some(), "asked again, with a question of its own");

    app.answer(true);
    assert_eq!(app.question(), None);
    assert_eq!(last_shown(), "guarded", "discarded, and gone");
}

#[guinea::test]
fn a_navigation_asks_its_guards_once(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Editor).unwrap();

    ENTERED.with(|entered| entered.set(0));
    app.navigate(Place::Guarded);
    app.answer(true);

    assert_eq!(last_shown(), "guarded");
    assert_eq!(
        ENTERED.with(Cell::get),
        1,
        "the answer lets the navigation that asked go on; it does not navigate there again"
    );
}

#[guinea::test]
fn a_first_place_that_fails_to_mount_is_shown_not_a_panic(h: &mut Harness) {
    SHOWN.with(|shown| shown.borrow_mut().clear());
    let app = Mounted::routed::<Shell>(h, Place::Unreachable).unwrap();

    assert!(SHOWN.with(|shown| shown.borrow().is_empty()));
    assert!(
        app.tree().find_text("Unreachable did not mount: the agent is not reachable").is_some(),
        "the window says why it is empty: {:#?}",
        app.tree()
    );
    assert_eq!(app.failure().as_deref(), Some("the agent is not reachable"));
}

#[guinea::test]
fn a_page_that_fails_to_mount_leaves_the_window_where_it_was(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Process { pid: 3 }).unwrap();

    app.navigate(Place::Unreachable);

    assert_eq!(app.route(), Place::Process { pid: 3 });
    assert_eq!(app.failure().as_deref(), Some("the agent is not reachable"));
    assert!(app.tree().find_text("process 3 at 0%").is_some(), "{:#?}", app.tree());
    assert!(app.is_mounted::<Process>());
}
