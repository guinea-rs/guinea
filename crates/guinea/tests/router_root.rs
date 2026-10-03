#![cfg(all(windows, feature = "winui"))]

//! A route tree mounted the way a window mounts it: `RouterRoot` on the
//! reactor's recording runtime, navigated from inside, and a guard's question
//! answered through the dialog it puts up, the way a user answers it.

use std::cell::{Cell, RefCell};

use guinea::enter::{Enter, EnterCx};
use guinea::feature::FeatureInitContext;
use guinea::router::NavigateHandle;
use guinea::winui::*;
use guinea_core::feature::Bound;
use guinea_core::scope::{Reducer, Scope};
use guinea_macros::routes;
use windows_reactor::test::{Command, EventId, NodeId, Pump, RecordingRuntime};
use windows_reactor::{ContentDialogResult, TextBlock, View};

thread_local! {
    static SHOWN: RefCell<Vec<String>> = RefCell::default();
    static NAV: RefCell<Option<NavigateHandle<WinUi, Route>>> = RefCell::default();
    static INSTALLED: Cell<Option<Scope>> = Cell::default();
    static ENTERED: Cell<usize> = Cell::default();
}

#[derive(Default, Clone, PartialEq, Debug)]
struct Load {
    percent: u32,
}

impl Reducer for Load {
    type Update = u32;

    fn reduce(&mut self, percent: u32) {
        self.percent = percent;
    }
}

#[derive(Default)]
struct Process {
    pid: u32,
}

#[derive(Default)]
struct Editor;

#[derive(Default)]
struct Guarded;

#[derive(Default)]
struct Unreachable;

struct Counted;

impl Enter for Counted {
    fn decide(_cx: &EnterCx<'_>) -> Verdict {
        ENTERED.with(|entered| entered.set(entered.get() + 1));
        Verdict::Allow
    }
}

routes! {
    Route {
        page(Process) link("/process/:pid") { pid: u32 }
        page(Editor) link("/editor")
        page(Guarded) guard(Counted) link("/guarded")
        page(Unreachable) link("/unreachable")
    }
}

#[page]
impl Page for Unreachable {
    type Params = UnreachableParams;

    fn install(_ctx: &FeatureInitContext, _params: &UnreachableParams) -> anyhow::Result<()> {
        anyhow::bail!("the agent is not reachable")
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        shown(cx, "unreachable".to_string())
    }
}

#[page]
impl Page for Process {
    type Params = ProcessParams;
    type Installs = Bound<Load>;

    fn install(ctx: &FeatureInitContext, _params: &ProcessParams) -> anyhow::Result<Bound<Load>> {
        INSTALLED.set(Some(ctx.scope));
        Ok(ctx.state::<Load>().seed(Load::default()).plain())
    }

    fn init(_ctx: &FeatureInitContext, params: &ProcessParams) -> Self {
        Self { pid: params.pid }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        let (load, _) = cx.read::<Load, _>();
        shown(cx, format!("process {} at {}%", self.pid, load.percent))
    }
}

#[page]
impl Page for Editor {
    type Params = EditorParams;

    fn leaving(&self) -> Verdict {
        Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep"))
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        shown(cx, "editor".to_string())
    }
}

#[page]
impl Page for Guarded {
    type Params = GuardedParams;

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        shown(cx, "guarded".to_string())
    }
}

fn shown<P: Page>(cx: &mut PageCx<'_, P>, text: String) -> View {
    NAV.with(|nav| *nav.borrow_mut() = Some(cx.use_navigate::<Route>()));
    SHOWN.with(|shown| shown.borrow_mut().push(text.clone()));
    TextBlock::new().text(text).into()
}

fn last_shown() -> String {
    SHOWN.with(|shown| shown.borrow().last().cloned().unwrap_or_default())
}

fn go(route: Route) {
    let nav = NAV.with(|nav| nav.borrow().clone()).expect("a page has drawn");
    nav.to(route);
}

fn mount(initial: Route) -> Pump<RecordingRuntime> {
    let mut pump = Pump::new(RecordingRuntime::default());
    pump.mount_view(View::component::<RouterRoot<Route>>(initial))
        .expect("the route tree mounts");
    settle(&mut pump);
    pump
}

fn settle(pump: &mut Pump<RecordingRuntime>) {
    loop {
        let events = pump.dispatch_events().expect("delivering events");
        let turns = pump.dispatch_components(64).expect("running components");
        if events + turns == 0 {
            return;
        }
    }
}

fn dialog(pump: &Pump<RecordingRuntime>) -> NodeId {
    pump.runtime()
        .commands()
        .iter()
        .flatten()
        .find_map(|command| match command {
            Command::Create { node, kind } if format!("{kind:?}") == "ContentDialog" => Some(*node),
            _ => None,
        })
        .expect("the root keeps a dialog for a guard's question")
}

fn answer(pump: &mut Pump<RecordingRuntime>, result: ContentDialogResult) {
    let dialog = dialog(pump);
    let revision = pump
        .event_revision(dialog, EventId::ContentDialogClosed)
        .expect("the dialog listens for its own closing");
    pump.runtime_mut()
        .complete_content_dialog(dialog, revision, result);
    settle(pump);
}

fn asking(pump: &Pump<RecordingRuntime>) -> bool {
    pump.runtime()
        .content_dialog(dialog(pump))
        .is_some_and(|dialog| dialog.desired_open)
}

#[test]
fn the_same_page_with_other_params_is_a_new_node_that_hears_its_own_scope() {
    let mut pump = mount(Route::Process { pid: 1 });
    assert_eq!(last_shown(), "process 1 at 0%");

    go(Route::Process { pid: 2 });
    settle(&mut pump);
    assert_eq!(last_shown(), "process 2 at 0%");

    let installed = INSTALLED.get().unwrap();
    installed.push::<Load>(40);
    settle(&mut pump);
    assert_eq!(
        last_shown(),
        "process 2 at 40%",
        "the page listens to the scope it was installed into, not the one before"
    );
}

#[test]
fn a_page_that_minds_being_left_asks_and_the_answer_decides() {
    let mut pump = mount(Route::Editor {});
    assert!(!asking(&pump));

    go(Route::Guarded {});
    settle(&mut pump);
    assert!(asking(&pump), "the question is on screen");
    assert_eq!(last_shown(), "editor");

    answer(&mut pump, ContentDialogResult::None);
    assert!(!asking(&pump));
    assert_eq!(last_shown(), "editor", "kept");

    go(Route::Guarded {});
    settle(&mut pump);
    assert!(asking(&pump), "asked again, with a question of its own");

    answer(&mut pump, ContentDialogResult::Primary);
    assert!(!asking(&pump));
    assert_eq!(last_shown(), "guarded", "discarded, and gone");
}

#[test]
fn a_navigation_asks_its_guards_once() {
    let mut pump = mount(Route::Editor {});

    ENTERED.with(|entered| entered.set(0));
    go(Route::Guarded {});
    settle(&mut pump);
    answer(&mut pump, ContentDialogResult::Primary);

    assert_eq!(last_shown(), "guarded");
    assert_eq!(
        ENTERED.with(Cell::get),
        1,
        "the root takes the route it arrived at; it does not navigate there again"
    );
}

#[test]
fn a_first_route_that_fails_to_install_is_shown_not_a_panic() {
    let pump = mount(Route::Unreachable {});

    assert!(SHOWN.with(|shown| shown.borrow().is_empty()));
    let said = pump
        .runtime()
        .commands()
        .iter()
        .flatten()
        .any(|command| format!("{command:?}").contains("the agent is not reachable"));
    assert!(said, "the window says why it is empty");
}

#[test]
fn a_page_that_fails_to_install_leaves_the_window_where_it_was() {
    let mut pump = mount(Route::Process { pid: 3 });

    go(Route::Unreachable {});
    settle(&mut pump);

    assert_eq!(last_shown(), "process 3 at 0%");
}
