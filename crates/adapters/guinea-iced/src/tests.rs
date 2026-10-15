//! The seam, without a window.
//!
//! Everything here is about what a frame mounts and what an envelope does
//! between the widget that made it and the node that receives it - which
//! needs a window's scope, a navigator and the shell's node store, and nothing
//! iced draws with.

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::scope::Scope;
use guinea_macros::reducer;

use crate::run::{Root, frame};
use crate::{
    Ask, Element, Envelope, Layout, LayoutCx, Navigator, Nodes, Observing, Page, PageCx, Routable, UpdateCx,
    Verdict, deliver_layout, deliver_page, envelope, layout, page,
};

/// A list the domain owns and replaces wholesale - the shape the coherence
/// rule exists for.
#[derive(Clone, Default, Debug)]
struct Domain(Vec<u32>);

#[derive(Clone)]
struct Replaced(Vec<u32>);

#[reducer]
fn domain(this: &mut Domain, update: Replaced) {
    this.0 = update.0;
}

#[derive(Clone, PartialEq, Debug, Routable)]
enum Place {
    Leaf,
    Counter,
    Other,
    Draft,
    Editor,
    Numbered { pid: u32 },
    Reader,
}

#[derive(Default)]
struct Shell {
    hits: usize,
}

#[derive(Clone)]
struct ShellMsg;

#[layout]
impl Layout for Shell {
    type Message = ShellMsg;

    fn update(&mut self, _message: ShellMsg, _cx: &mut UpdateCx<'_, Self>) {
        self.hits += 1;
    }

    fn view<'a>(&'a self, cx: &mut LayoutCx<'a, Self>) -> Element<'a, Envelope> {
        let nav = cx.navigate::<Place>();
        cx.outlet(&nav, |place, cx| match place {
            Place::Leaf => cx.mount::<Leaf>(&()).keep(),
            Place::Counter => cx.mount::<Counter>(&()),
            Place::Other => cx.mount::<Other>(&()),
            Place::Draft => cx.mount::<Draft>(&()),
            Place::Editor => cx.mount::<Editor>(&()).keep(),
            Place::Numbered { pid } => cx.mount::<Numbered>(pid),
            Place::Reader => cx.mount::<Reader>(&()),
        })
    }
}

/// The page *is* its state - there is no second type for it.
#[derive(Default)]
struct Leaf {
    picked: usize,
    /// Every message that reached `update`, in order - what proves there is
    /// one mutation point rather than two.
    log: Vec<&'static str>,
}

#[derive(Clone)]
enum LeafMsg {
    Pick(usize),
    Clamp(usize),
}

/// The coherence rule: a pure translation, named at exactly one place - the
/// `cx.on` below. Forget that line and this is a function nobody calls, which
/// the dead-code lint says out loud.
fn list_replaced(update: &Replaced) -> Option<LeafMsg> {
    Some(LeafMsg::Clamp(update.0.len()))
}

#[page]
impl Page for Leaf {
    type Message = LeafMsg;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.state::<Domain>().seed(Domain(vec![1, 2, 3])).plain();
        Ok(Self::default())
    }

    fn observes(cx: &Observing<'_, LeafMsg>) {
        cx.on::<Domain>(list_replaced);
    }

    fn update(&mut self, message: LeafMsg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            LeafMsg::Pick(row) => {
                self.picked = row;
                self.log.push("pick");
            }
            LeafMsg::Clamp(len) => {
                self.picked = self.picked.min(len.saturating_sub(1));
                self.log.push("clamp");
            }
        }
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, LeafMsg> {
        iced::widget::text("").into()
    }
}

/// Counts, and is not kept: leaving it is the end of it.
#[derive(Default)]
struct Counter {
    hits: usize,
}

#[derive(Clone)]
struct Hit;

#[page]
impl Page for Counter {
    type Message = Hit;

    fn update(&mut self, _message: Hit, _cx: &mut UpdateCx<'_, Self>) {
        self.hits += 1;
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, Hit> {
        iced::widget::text("").into()
    }
}

/// Everything `#[page]` writes for a node that keeps nothing and says
/// nothing: `Params`, `Message` and the empty `update`.
#[derive(Default)]
struct Other;

#[page]
impl Page for Other {
    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, Self::Message> {
        iced::widget::text("").into()
    }
}

/// Minds being left, whatever it holds: every attempt to go puts a question.
#[derive(Default)]
struct Draft;

#[page]
impl Page for Draft {
    fn leaving(&self) -> Verdict {
        Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep editing"))
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, Self::Message> {
        iced::widget::text("").into()
    }
}

#[derive(Default)]
struct Editor {
    dirty: bool,
}

#[derive(Clone)]
struct Typed;

#[page]
impl Page for Editor {
    type Message = Typed;

    fn leaving(&self) -> Verdict {
        match self.dirty {
            true => Verdict::ask(Ask::new("Discard the edit?", "Discard", "Keep")),
            false => Verdict::Allow,
        }
    }

    fn update(&mut self, _message: Typed, _cx: &mut UpdateCx<'_, Self>) {
        self.dirty = true;
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, Typed> {
        iced::widget::text("").into()
    }
}

#[derive(Default)]
struct Numbered {
    pid: u32,
    picked: Option<u32>,
}

#[derive(Clone)]
struct PickHere;

#[page]
impl Page for Numbered {
    type Params = u32;
    type Message = PickHere;

    fn install(_ctx: &FeatureInitContext, pid: &u32) -> anyhow::Result<Self> {
        Ok(Self { pid: *pid, picked: None })
    }

    fn update(&mut self, _message: PickHere, _cx: &mut UpdateCx<'_, Self>) {
        self.picked = Some(self.pid);
    }

    fn view(&self, _cx: &mut PageCx<'_, Self>) -> Element<'_, PickHere> {
        iced::widget::text("").into()
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Shown(u32);

#[reducer]
fn shown(this: &mut Shown, to: u32) {
    this.0 = to;
}

struct Showing;

impl guinea_app::app::Plugin for Showing {
    const ID: &'static str = "showing";
    type Exports = (Shown,);

    fn build(self, app: &mut guinea_app::app::PluginBuilder) -> anyhow::Result<()> {
        app.state::<Shown>().seed(Shown(7)).plain();
        Ok(())
    }
}

thread_local! {
    static SEEN: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// A plugin's shortcut, written once for every backend.
trait Sees: guinea_app::feature::Reads {
    fn shown(&mut self) -> u32 {
        self.read::<Shown>().0.0
    }
}

impl<C: guinea_app::feature::Reads> Sees for C {}

#[derive(Default)]
struct Reader;

#[page]
impl Page for Reader {
    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Self::Message> {
        SEEN.set(Some(cx.shown()));
        iced::widget::text("").into()
    }
}

/// A window's scope, its navigator and the store the shell would own, drawn
/// the way the shell draws them.
struct Window {
    host: FeatureHost,
    nodes: Nodes,
    nav: Navigator<Place>,
}

impl Window {
    fn at(place: Place) -> Self {
        Self::hosted(place, FeatureHost::detached())
    }

    fn hosted(place: Place, host: FeatureHost) -> Self {
        let nav = Navigator::open(host.scope(), place);
        let window = Window {
            host,
            nodes: Nodes::default(),
            nav,
        };
        window.draw();
        window
    }

    fn draw(&self) {
        drop(frame(self.host.scope(), &Root::of::<Shell>(), &self.nodes));
    }

    fn send(&mut self, envelope: Envelope) {
        envelope::settle(&mut self.nodes, Some(&self.nav), envelope);
        self.draw();
    }

    fn go(&mut self, place: Place) {
        let _moved = self.nav.to(place);
        self.draw();
    }

    fn answer(&mut self, allowed: bool) {
        self.send(Envelope::answer(allowed));
    }

    /// The scope the `N` drawn in this window was mounted into.
    fn scope<N: 'static>(&self) -> Option<Scope> {
        self.host
            .scope()
            .subtree()
            .into_iter()
            .find(|scope| guinea_router::mount::name(*scope) == Some(std::any::type_name::<N>()))
    }

    fn node<N: 'static>(&self) -> Option<&N> {
        self.nodes.get::<N>(self.scope::<N>()?)
    }

    fn to<N: Page>(&self, message: N::Message) -> Envelope {
        let scope = self.scope::<N>().expect("mounted");
        Envelope::node(scope, deliver_page::<N>, Box::new(message))
    }
}

#[test]
fn what_is_mounted_says_where_it_was_written() {
    let window = Window::at(Place::Leaf);
    let root = guinea_app::app::roots::of(window.host.scope()).map(|root| root.get());

    let written: Option<Vec<&str>> = root.and_then(guinea_router::observability::window).map(|view| {
        view.mounted
            .iter()
            .filter(|mounted| mounted.written.is_some_and(|at| at.file.ends_with("tests.rs")))
            .map(|mounted| mounted.name)
            .collect()
    });

    assert_eq!(written, Some(vec!["Shell", "Leaf"]));
}

#[test]
fn a_message_reaches_its_own_node_and_no_other() {
    let mut window = Window::at(Place::Leaf);

    window.send(window.to::<Leaf>(LeafMsg::Pick(2)));

    assert_eq!(window.node::<Leaf>().expect("a leaf is mounted").picked, 2);
    assert_eq!(
        window.node::<Shell>().expect("a layout is mounted").hits,
        0,
        "the layout above never sees its child's messages - which is why \
         adding a page costs it no edit"
    );
}

#[test]
fn a_layout_s_own_message_reaches_the_layout() {
    let mut window = Window::at(Place::Leaf);
    let shell = window.scope::<Shell>().expect("mounted");

    window.send(Envelope::node(shell, deliver_layout::<Shell>, Box::new(ShellMsg)));

    assert_eq!(window.node::<Shell>().expect("mounted").hits, 1);
}

#[test]
fn an_observer_translates_rather_than_mutates() {
    let mut window = Window::at(Place::Leaf);
    window.send(window.to::<Leaf>(LeafMsg::Pick(2)));

    window.scope::<Leaf>().expect("a leaf is mounted").push::<Domain>(Replaced(vec![9]));

    assert_eq!(
        window.node::<Leaf>().expect("mounted").picked,
        2,
        "observing produced a message, and a message is not a mutation"
    );

    window.send(Envelope::settled());

    let leaf = window.node::<Leaf>().expect("mounted");
    assert_eq!(leaf.picked, 0, "the node clamped itself, through update");
    assert_eq!(
        leaf.log,
        ["pick", "clamp"],
        "both changes went through the same update - there is one mutation point"
    );
}

#[test]
fn a_page_navigated_to_is_drawn_in_the_same_frame() {
    let mut window = Window::at(Place::Leaf);

    window.go(Place::Other);

    assert!(window.node::<Other>().is_some(), "mounted and in the store after one frame");
}

#[test]
fn a_kept_page_comes_back_to_the_state_it_left() {
    let mut window = Window::at(Place::Leaf);
    window.send(window.to::<Leaf>(LeafMsg::Pick(2)));

    window.go(Place::Other);
    window.go(Place::Leaf);

    assert_eq!(
        window.node::<Leaf>().expect("mounted").picked,
        2,
        "the node slept with its scope and came back, rather than one install built"
    );
}

#[test]
fn a_page_not_kept_starts_fresh() {
    let mut window = Window::at(Place::Counter);
    window.send(window.to::<Counter>(Hit));
    assert_eq!(window.node::<Counter>().expect("mounted").hits, 1);

    window.go(Place::Other);
    window.go(Place::Counter);

    assert_eq!(
        window.node::<Counter>().expect("mounted").hits,
        0,
        "keeping state across navigation is opt-in, and this mount did not"
    );
}

#[test]
fn a_page_kept_once_is_asked_again_the_next_time() {
    let mut window = Window::at(Place::Draft);

    let _moved = window.nav.to(Place::Other);
    assert!(window.nav.pending().is_some());
    window.answer(false);
    assert!(window.nav.pending().is_none());
    assert_eq!(window.nav.current(), Some(Place::Draft));

    let _moved = window.nav.to(Place::Other);
    assert!(
        window.nav.pending().is_some(),
        "the second attempt is a question of its own, not the first one's answer"
    );
    window.answer(true);
    assert_eq!(window.nav.current(), Some(Place::Other));
}

#[test]
fn a_shortcut_written_over_reads_reads_in_a_view() {
    let runtime = guinea_app::app::GuineaApp::new()
        .plugin(Showing)
        .install()
        .expect("install");
    SEEN.set(None);

    let _window = Window::hosted(Place::Reader, FeatureHost::under(&runtime.context()));

    assert_eq!(SEEN.get(), Some(7));
}

#[test]
fn a_message_for_a_node_that_left_is_dropped() {
    let mut window = Window::at(Place::Counter);
    let late = window.to::<Counter>(Hit);

    window.go(Place::Other);
    window.send(late);

    assert!(
        window.node::<Counter>().is_none(),
        "delivering it would have made a node for a page that is not there"
    );
}

#[test]
fn a_kept_node_that_came_back_dirty_still_minds_being_left() {
    let mut window = Window::at(Place::Editor);
    window.send(window.to::<Editor>(Typed));

    let _moved = window.nav.to(Place::Other);
    assert!(window.nav.pending().is_some(), "a dirty editor asks");
    window.answer(true);

    window.go(Place::Editor);
    assert!(window.node::<Editor>().expect("mounted").dirty, "the kept node came back as it was left");

    let _moved = window.nav.to(Place::Other);
    assert!(window.nav.pending().is_some(), "the node on screen is dirty, so leaving it asks");
}

#[test]
fn a_message_for_a_node_mounted_again_with_other_params_is_dropped() {
    let mut window = Window::at(Place::Numbered { pid: 1 });
    let from_pid_1 = window.to::<Numbered>(PickHere);

    window.go(Place::Numbered { pid: 2 });
    assert_eq!(window.node::<Numbered>().expect("mounted").pid, 2);

    window.send(from_pid_1);

    assert_eq!(
        window.node::<Numbered>().expect("mounted").picked,
        None,
        "the pick was made on the page for pid 1, which is gone"
    );
}
