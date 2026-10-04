//! The seam, without a window.
//!
//! Everything here is about what an envelope does between the widget that made
//! it and the node that receives it - which needs a router, two scopes and the
//! shell's node store, and nothing iced draws with.

use std::any::Any;

use guinea_app::feature::{FeatureHost, FeatureInitContext};
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::Reducer;
use guinea_router::router::{RouteChain, Router, SegmentEntry};

use crate::{
    Ask, Verdict,
    Element, Envelope, Iced, Layout, LayoutCx, Nodes, Observing, Page, PageCx, UpdateCx, deliver_page,
    envelope, layout, layout_entry, page, segment_entry,
};

/// A list the domain owns and replaces wholesale - the shape the coherence
/// rule exists for.
#[derive(Clone, Default, Debug)]
struct Domain(Vec<u32>);

#[derive(Clone)]
struct Replaced(Vec<u32>);

impl Reducer for Domain {
    type Update = Replaced;

    fn reduce(&mut self, update: Replaced) {
        self.0 = update.0;
    }
}

#[derive(Default)]
struct Shell {
    hits: usize,
}

#[derive(Clone)]
struct ShellMsg;

#[layout]
impl Layout for Shell {
    type Params = ();
    type Message = ShellMsg;

    fn update(&mut self, _message: ShellMsg, _cx: &mut UpdateCx<'_, Self>) {
        self.hits += 1;
    }

    fn view<'a>(&'a self, cx: &mut LayoutCx<'a, Self>) -> Element<'a, Envelope> {
        cx.outlet()
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
    const CACHE_STATE_IN_MEMORY: bool = true;

    type Params = ();
    type Message = LeafMsg;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        ctx.state::<Domain>().seed(Domain(vec![1, 2, 3])).plain();
        Ok(())
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

/// A different page for the same position in the chain - and everything
/// `#[page]` writes for a node that keeps nothing and says nothing: `Params`,
/// `Message` and the empty `update`.
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

const WITH_DRAFT: [SegmentEntry<Iced>; 2] = [layout_entry::<Shell>(), segment_entry::<Draft>()];

#[derive(Clone, PartialEq, Debug)]
enum Place {
    Draft,
    Other,
}

impl RouteChain<Iced> for Place {
    fn chain(&self) -> &'static [SegmentEntry<Iced>] {
        match self {
            Place::Draft => &WITH_DRAFT,
            Place::Other => &WITH_OTHER,
        }
    }

    fn params(&self) -> Vec<Box<dyn Any>> {
        params()
    }

    fn name(&self) -> &'static str {
        match self {
            Place::Draft => "Draft",
            Place::Other => "Other",
        }
    }
}

const WITH_LEAF: [SegmentEntry<Iced>; 2] = [layout_entry::<Shell>(), segment_entry::<Leaf>()];
const WITH_OTHER: [SegmentEntry<Iced>; 2] = [layout_entry::<Shell>(), segment_entry::<Other>()];

fn params() -> Vec<Box<dyn Any>> {
    vec![Box::new(()), Box::new(())]
}

/// The router and the store the shell would own, kept together because every
/// step of a navigation touches both.
struct Mounted {
    router: std::rc::Rc<Router<Iced>>,
    nodes: Nodes,
}

impl Mounted {
    fn at(chain: &'static [SegmentEntry<Iced>]) -> Self {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        Self::hosted(chain, FeatureHost::detached(token))
    }

    fn hosted(chain: &'static [SegmentEntry<Iced>], host: FeatureHost) -> Self {
        let router = std::rc::Rc::new(Router::<Iced>::new(host));
        let mut mounted = Mounted {
            router,
            nodes: Nodes::default(),
        };
        mounted.activate(chain);
        mounted
    }

    fn activate(&mut self, chain: &'static [SegmentEntry<Iced>]) {
        self.router.activate(chain, params()).expect("activate");
        self.nodes.sync(chain);
    }

    fn send(&mut self, envelope: Envelope) {
        envelope::settle(&self.router, &mut self.nodes, envelope);
    }

    fn leaf(&self) -> &Leaf {
        self.nodes.get::<Leaf>(1).expect("a leaf is mounted")
    }
}

fn to_leaf(message: LeafMsg) -> Envelope {
    Envelope::new(1, deliver_page::<Leaf>, Box::new(message))
}

#[test]
fn a_message_reaches_its_own_node_and_no_other() {
    let mut mounted = Mounted::at(&WITH_LEAF);

    mounted.send(to_leaf(LeafMsg::Pick(2)));

    assert_eq!(mounted.leaf().picked, 2);
    assert_eq!(
        mounted
            .nodes
            .get::<Shell>(0)
            .expect("a layout is mounted")
            .hits,
        0,
        "the layout above never sees its child's messages - which is why \
         adding a page costs it no edit"
    );
}

#[test]
fn an_observer_translates_rather_than_mutates() {
    let mut mounted = Mounted::at(&WITH_LEAF);
    mounted.send(to_leaf(LeafMsg::Pick(2)));

    // The domain replaces the list with a shorter one, leaving the selection
    // past the end.
    mounted
        .router
        .scope_at(1)
        .expect("a leaf is mounted")
        .push::<Domain>(Replaced(vec![9]));

    assert_eq!(
        mounted.leaf().picked,
        2,
        "observing produced a message, and a message is not a mutation"
    );

    mounted.send(Envelope::settled());

    assert_eq!(
        mounted.leaf().picked,
        0,
        "the node clamped itself, through update"
    );
    assert_eq!(
        mounted.leaf().log,
        ["pick", "clamp"],
        "both changes went through the same update - there is one mutation point"
    );
}

#[test]
fn a_cached_page_comes_back_to_the_state_it_left() {
    let mut mounted = Mounted::at(&WITH_LEAF);
    mounted.send(to_leaf(LeafMsg::Pick(2)));

    // What navigating away and back does: the chain is torn down to nothing
    // and installed again.
    mounted.activate(&WITH_OTHER);
    mounted.activate(&WITH_LEAF);

    assert_eq!(
        mounted.leaf().picked,
        2,
        "the node the shell kept came back, rather than the one init built"
    );
}

#[test]
fn a_page_that_did_not_ask_starts_fresh() {
    let mut mounted = Mounted::at(&WITH_LEAF);
    mounted.send(Envelope::new(0, crate::deliver_layout::<Shell>, Box::new(ShellMsg)));

    assert_eq!(mounted.nodes.get::<Shell>(0).expect("mounted").hits, 1);

    mounted.activate(&WITH_OTHER);
    mounted.activate(&WITH_LEAF);

    assert_eq!(
        mounted.nodes.get::<Shell>(0).expect("mounted").hits,
        0,
        "keeping state across navigation is opt-in, and this layout did not"
    );
}

#[test]
fn a_page_kept_once_is_asked_again_the_next_time() {
    let mut mounted = Mounted::at(&WITH_LEAF);
    mounted.router.navigate(Place::Draft).expect("to the draft");
    mounted.nodes.sync(&WITH_DRAFT);

    mounted.router.navigate(Place::Other).expect("asked");
    assert!(mounted.router.pending().is_some());
    mounted.router.answer(false);
    assert!(mounted.router.pending().is_none());
    assert_eq!(mounted.router.current_route::<Place>(), Some(Place::Draft));

    mounted.router.navigate(Place::Other).expect("asked again");
    assert!(
        mounted.router.pending().is_some(),
        "the second attempt is a question of its own, not the first one's answer"
    );
    mounted.router.answer(true);
    assert_eq!(mounted.router.current_route::<Place>(), Some(Place::Other));
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Shown(u32);

impl Reducer for Shown {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
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

guinea_macros::app! {
    App {
        installs { Showing }
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

const WITH_READER: [SegmentEntry<Iced>; 2] = [layout_entry::<Shell>(), segment_entry::<Reader>()];

#[test]
fn a_shortcut_written_over_reads_reads_in_a_view() {
    let runtime = guinea_app::app::GuineaApp::new()
        .plugin(Showing)
        .install(UiThreadToken::dangerously_create_token_unchecked())
        .expect("install");
    let mounted = Mounted::hosted(&WITH_READER, FeatureHost::under(&runtime.context()));

    drop(mounted.router.render(&mounted.nodes));

    assert_eq!(SEEN.get(), Some(7));
}

#[test]
fn a_message_for_a_node_that_left_the_chain_is_dropped() {
    let mut mounted = Mounted::at(&WITH_LEAF);
    mounted.activate(&WITH_OTHER);

    // A click that raced a navigation. The position still exists; what sits
    // there is a different page.
    mounted.send(to_leaf(LeafMsg::Pick(9)));

    assert!(
        mounted.nodes.get::<Leaf>(1).is_none(),
        "delivering it would have made a node for a page that is not there"
    );
}
