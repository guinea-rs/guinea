//! A page or a layout with no window.
//!
//! Mounted on reactor's recording runtime: the native tree it would build is
//! read back as data, and its messages and clicks are delivered the way the
//! window would deliver them. It lives in a segment of guinea-app's
//! `Harness`, so everything it sets off runs in the seed's order and on the
//! test's clock.
//!
//! A page that reads what a layout above it exports is mounted below that
//! layout: the layout into `h.segment()`, the page into `h.child()`.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::app::{Act, Harness, Segment};
use guinea_app::feature::FeatureInitContext;
use guinea_core::feature::Dispatch;
use guinea_core::mark::Mark;
use guinea_core::scope::{Reducer, Scope};
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, RouteSink, Router, SegmentEntry, SegmentProps,
};
use windows_reactor::{
    Border, Callback, ComponentHost, ComponentNode, CompositionHostEvent, ContentDialogResult,
    EventDispatch, EventId, EventPayload, EventValue, ImperativeRequest, Mutation, ObjectType,
    Observation, PointerEventInfo, Property, RealizationRequest,
    RealizedContainer, RecordingAdapter, RelationId, RetainedGraph, SelectionChange, View,
    component,
};

pub use windows_reactor::{ObjectId as NodeId, PropertyId, PropertyValue};

use crate::mark::MarkExt;
use crate::winui::{
    Layout, LayoutNode, Page, PageNode, Rooted, RouterRoot, Shown, Signal, WinUi, install_layout,
    install_page, layout_entry, nav_context, route_context, segment_entry,
};

/// How many messages one pass may run before it looks again.
const TURNS: usize = 64;

/// What the host calls the one component it mounts.
const ROOT: &str = "mounted";

/// Every place a control holds a child, in the order they read on screen - a
/// `NavigationView`'s pane before its content, a header before what it heads.
const RELATIONS: &[RelationId] = &[
    RelationId::Children,
    RelationId::LeftHeader,
    RelationId::Header,
    RelationId::RightHeader,
    RelationId::PaneCustomContent,
    RelationId::MenuItems,
    RelationId::FooterMenuItems,
    RelationId::PaneFooter,
    RelationId::Pane,
    RelationId::Content,
    RelationId::OnContent,
    RelationId::OffContent,
    RelationId::Items,
    RelationId::TabItems,
    RelationId::PrimaryCommands,
    RelationId::SecondaryCommands,
    RelationId::Roots,
];

/// The controls a click turns over, by kind: the event that says so, the
/// state it turns, and whether a click turns it back. A radio button does
/// not: a click only ever checks it.
const FLIPS: &[(ObjectType, EventId, PropertyId, bool)] = &[
    (ObjectType::ToggleSwitch, EventId::Toggled, PropertyId::IsOn, true),
    (ObjectType::CheckBox, EventId::IsCheckedChanged, PropertyId::IsChecked, true),
    (ObjectType::ToggleButton, EventId::IsCheckedChanged, PropertyId::IsChecked, true),
    (ObjectType::RadioButton, EventId::Checked, PropertyId::IsChecked, false),
];

type Sender<M> = Rc<dyn Fn(Signal<M>) -> bool>;

thread_local! {
    static SENDERS: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
    static CHAINS: RefCell<HashMap<(TypeId, usize), &'static [SegmentEntry<WinUi>]>> =
        RefCell::new(HashMap::new());
    static ROUTER: RefCell<Option<Rc<Router<WinUi>>>> = const { RefCell::new(None) };
    static NAV: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
}

/// The router a [`RouterRoot`] built, kept for [`Mounted::routed`].
pub(crate) fn remember_router(router: &Rc<Router<WinUi>>) {
    ROUTER.with(|kept| *kept.borrow_mut() = Some(router.clone()));
}

/// The handle a [`RouterRoot`] last handed its tree, for
/// [`Mounted::navigate`] to go the same way a page does.
pub(crate) fn remember_nav<R: 'static>(nav: &NavigateHandle<WinUi, R>) {
    NAV.with(|kept| *kept.borrow_mut() = Some(Box::new(nav.clone())));
}

/// What a mounted page's or layout's component answers to, kept as it is
/// created.
pub(crate) fn remember<S: 'static, M: 'static>(send: impl Fn(Signal<M>) -> bool + 'static) {
    let send: Sender<M> = Rc::new(send);
    SENDERS.with(|senders| {
        senders.borrow_mut().insert(TypeId::of::<S>(), Box::new(send));
    });
}

fn sender<S: 'static, M: 'static>() -> Sender<M> {
    SENDERS.with(|senders| {
        senders
            .borrow()
            .get(&TypeId::of::<S>())
            .and_then(|send| send.downcast_ref::<Sender<M>>())
            .cloned()
            .unwrap_or_else(|| panic!("{} is not mounted", std::any::type_name::<S>()))
    })
}

/// A chain `depth` long whose every entry is this segment's own: a segment
/// below `depth - 1` others names itself by its place in the chain, and the
/// segments above it have no entries of their own here. A layout's chain goes
/// on one further, to the [`Outlet`] it draws its outlet with. Made once per
/// segment and depth.
fn chain<S: Mountable<K>, K>(depth: usize) -> &'static [SegmentEntry<WinUi>] {
    CHAINS.with(|chains| {
        *chains
            .borrow_mut()
            .entry((TypeId::of::<S>(), depth))
            .or_insert_with(|| {
                let entries: Vec<SegmentEntry<WinUi>> = (0..depth)
                    .map(|_| S::entry())
                    .chain(S::HAS_OUTLET.then_some(OUTLET))
                    .collect();
                Box::leak(entries.into_boxed_slice())
            })
    })
}

/// What a mounted layout draws in its outlet: an empty border marked
/// `Outlet`, to find where the page below would go.
#[derive(Clone, Copy, Debug)]
pub struct Outlet;

impl Mark for Outlet {
    fn name(&self) -> &'static str {
        "Outlet"
    }
}

struct MountOutlet;

impl Mount<WinUi> for MountOutlet {
    fn view<'a>(&self, _props: SegmentProps<WinUi>, _nodes: &'a ()) -> View {
        Border::new().mark(Outlet).into()
    }
}

const OUTLET: SegmentEntry<WinUi> =
    SegmentEntry::new::<Outlet>(|_, _| Ok(()), |_, _| true, |_| {}, &MountOutlet as &dyn Mount<WinUi>, false);

/// A host with no window for `root`, recording every batch it applies - a
/// list says how long it is only in the batch that sets its source.
fn host(root: ComponentNode) -> Result<ComponentHost<RecordingAdapter>, String> {
    let mut adapter = RecordingAdapter::new();
    adapter.record_batches(true);

    ComponentHost::mount(adapter, [root]).map_err(|refused| format!("{refused:?}"))
}

/// A drag with the left button, for [`Mounted::drag`]: down on an element,
/// moved by `dx`, `dy` in even steps, and up.
///
/// There is no layout under the harness, so the coordinates are what the
/// drag says: `x`, `y` start where [`from`](Self::from) puts them inside the
/// element and `window_x`, `window_y` where [`window_from`](Self::window_from)
/// puts them in the window, and both move by the same steps.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    dx: f64,
    dy: f64,
    steps: usize,
    from: (f64, f64),
    window: (f64, f64),
    lost: bool,
}

impl Drag {
    /// By `dx`, `dy`, in four moves, from a point one unit inside the
    /// element's corner - and the same point of the window.
    pub fn by(dx: f64, dy: f64) -> Self {
        Self {
            dx,
            dy,
            steps: 4,
            from: (1.0, 1.0),
            window: (1.0, 1.0),
            lost: false,
        }
    }

    /// In `steps` moves rather than four.
    pub fn steps(self, steps: usize) -> Self {
        Self {
            steps: steps.max(1),
            ..self
        }
    }

    /// Where inside the element the button goes down: `x`, `y`.
    pub fn from(self, x: f64, y: f64) -> Self {
        Self {
            from: (x, y),
            ..self
        }
    }

    /// Where in the window the button goes down: `window_x`, `window_y`.
    pub fn window_from(self, x: f64, y: f64) -> Self {
        Self {
            window: (x, y),
            ..self
        }
    }

    /// Ends with the capture lost and no release - the system taking the
    /// pointer away mid-drag.
    pub fn lost(self) -> Self {
        Self { lost: true, ..self }
    }

    fn at(&self, step: usize, held: bool) -> PointerEventInfo {
        let part = step as f64 / self.steps as f64;
        let (dx, dy) = (self.dx * part, self.dy * part);

        PointerEventInfo {
            x: self.from.0 + dx,
            y: self.from.1 + dy,
            window_x: self.window.0 + dx,
            window_y: self.window.1 + dy,
            is_left_button_pressed: held,
            ..Default::default()
        }
    }

    fn pressed(&self, captures: bool) -> PointerEventInfo {
        PointerEventInfo {
            capture_succeeded: Some(captures),
            is_captured: captures,
            ..self.at(0, true)
        }
    }

    fn moved(&self, step: usize) -> PointerEventInfo {
        self.at(step, true)
    }

    fn released(&self) -> PointerEventInfo {
        self.at(self.steps, false)
    }
}

/// A page or a layout - what [`Mounted`] mounts. `K` only tells the two apart,
/// and is always inferred.
pub trait Mountable<K>: 'static {
    type Params: 'static;
    type Message: 'static;

    #[doc(hidden)]
    const HAS_OUTLET: bool;

    #[doc(hidden)]
    fn install(cx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<()>;

    #[doc(hidden)]
    fn entry() -> SegmentEntry<WinUi>;

    #[doc(hidden)]
    fn component(props: SegmentProps<WinUi>) -> View;
}

/// [`Mountable`]'s `K` for a page.
pub enum AsPage {}

/// [`Mountable`]'s `K` for a layout.
pub enum AsLayout {}

impl<P: Page> Mountable<AsPage> for P {
    type Params = P::Params;
    type Message = P::Message;

    const HAS_OUTLET: bool = false;

    fn install(cx: &FeatureInitContext, params: &P::Params) -> anyhow::Result<()> {
        install_page::<P>(cx, params)
    }

    fn entry() -> SegmentEntry<WinUi> {
        segment_entry::<P>()
    }

    fn component(props: SegmentProps<WinUi>) -> View {
        View::component::<PageNode<P>>(props)
    }
}

impl<L: Layout> Mountable<AsLayout> for L {
    type Params = L::Params;
    type Message = L::Message;

    const HAS_OUTLET: bool = true;

    fn install(cx: &FeatureInitContext, params: &L::Params) -> anyhow::Result<()> {
        install_layout::<L>(cx, params)
    }

    fn entry() -> SegmentEntry<WinUi> {
        layout_entry::<L>()
    }

    fn component(props: SegmentProps<WinUi>) -> View {
        View::component::<LayoutNode<L>>(props)
    }
}

/// One element of what a page drew, as a snapshot keeps it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Node {
    /// Where it is, for [`Mounted::property`] and [`Mounted::at`]. Not part
    /// of a snapshot: it changes from run to run.
    #[serde(skip)]
    pub at: NodeId,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Its `AutomationId` - the mark every page and layout carries on its
    /// border, and any the page set itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

impl Node {
    /// The first element here, depth first, that shows `text`.
    pub fn find_text(&self, text: &str) -> Option<&Node> {
        self.first(&|node| node.text.as_deref() == Some(text))
    }

    /// The first element here, depth first, that carries `mark`.
    pub fn find(&self, mark: impl Mark) -> Option<&Node> {
        let name = mark.name();
        self.first(&|node| node.id.as_deref() == Some(name))
    }

    fn first(&self, test: &dyn Fn(&Node) -> bool) -> Option<&Node> {
        if test(self) {
            return Some(self);
        }

        self.children.iter().find_map(|child| child.first(test))
    }
}

/// A page or a layout mounted with no window, in a segment of a [`Harness`].
///
/// A layout draws an [`Outlet`] where the page below it would go.
pub struct Mounted<'h, S> {
    harness: &'h Harness,
    host: ComponentHost<RecordingAdapter>,
    /// How many of the batches the adapter recorded have been read.
    read: usize,
    /// The items brought into view so far, by list and index.
    realized: HashMap<(NodeId, usize), NodeId>,
    /// How many items each list holds, as it last said.
    counts: HashMap<NodeId, usize>,
    /// Where it asked to go, when mounted [at a route](Self::mount_at): an
    /// `Rc<RefCell<Vec<R>>>` for that route type.
    navigated: Option<Box<dyn Any>>,
    /// The router, when the whole route tree is [mounted](Self::routed).
    router: Option<Rc<Router<WinUi>>>,
    /// The sizes the test laid marked elements out at, in the order given.
    sizes: Vec<(&'static str, f64, f64)>,
    /// The size each composition host was last told, by its observation.
    told: HashMap<(NodeId, u64), (f64, f64)>,
    kind: PhantomData<S>,
    /// The segment it is mounted into, kept for as long as it is: dropped
    /// after the page, the way leaving a page tears down its scope.
    segment: Segment<'h>,
}

impl<S> Drop for Mounted<'_, S> {
    fn drop(&mut self) {
        if self.router.is_some() {
            NAV.with(|kept| kept.borrow_mut().take());
        }
    }
}

impl<'h, S: 'static> Mounted<'h, S> {
    /// Installs `S` into `segment` - what it `Installs`, and the node it
    /// starts as - and mounts it, the way a navigation to it would.
    pub fn mount<K>(
        segment: Segment<'h>,
        params: <S as Mountable<K>>::Params,
    ) -> anyhow::Result<Self>
    where
        S: Mountable<K>,
    {
        Self::mount_with(segment, params, |view| view)
    }

    /// [`mount`](Self::mount), with the view handed to `wrap` first - for
    /// what a layout above it would give it, such as a context:
    /// `|page| provide(&SCHEME, scheme, page)`.
    pub fn mount_with<K>(
        segment: Segment<'h>,
        params: <S as Mountable<K>>::Params,
        wrap: impl FnOnce(View) -> View,
    ) -> anyhow::Result<Self>
    where
        S: Mountable<K>,
    {
        let cx = segment.context();
        S::install(cx, &params)?;

        let mut scopes: Vec<Scope> = cx.scope.ancestors();
        scopes.truncate(cx.cursor);
        scopes.reverse();
        scopes.push(cx.scope);
        let depth = scopes.len();

        let props = SegmentProps {
            chain: chain::<S, K>(depth),
            scopes: Rc::new(scopes),
            cursor: depth - 1,
        };

        let host = host(component::<Shown>(ROOT, wrap(S::component(props))))
            .map_err(|refused| anyhow::anyhow!("mounting {}: {refused}", std::any::type_name::<S>()))?;

        let mut mounted = Self {
            harness: segment.harness(),
            host,
            read: 0,
            realized: HashMap::new(),
            counts: HashMap::new(),
            navigated: None,
            router: None,
            sizes: Vec::new(),
            told: HashMap::new(),
            kind: PhantomData,
            segment,
        };
        mounted.settle();

        Ok(mounted)
    }

    /// [`mount`](Self::mount), with `route` as where the application is: what
    /// `use_route::<R>()` returns. `use_navigate::<R>()` moves nothing - where
    /// it was asked to go is kept for [`navigated`](Self::navigated), and the
    /// route `use_route` returns stays `route`.
    pub fn mount_at<K, R>(
        segment: Segment<'h>,
        params: <S as Mountable<K>>::Params,
        route: R,
    ) -> anyhow::Result<Self>
    where
        S: Mountable<K>,
        R: RouteChain<WinUi> + Clone + PartialEq + 'static,
    {
        let navigated: Rc<RefCell<Vec<R>>> = Rc::default();

        let asked = navigated.clone();
        let nav = NavigateHandle::recording(RouteSink::new(move |to: R| {
            asked.borrow_mut().push(to);
        }));

        let mut mounted = Self::mount_with(segment, params, |view| {
            let view = windows_reactor::provide(route_context::<R>(), Some(route), view);
            windows_reactor::provide(nav_context::<R>(), Some(nav), view)
        })?;
        mounted.navigated = Some(Box::new(navigated));

        Ok(mounted)
    }

    /// The segment it is mounted into: to install more there, or to read
    /// what a page there reads.
    pub fn segment(&self) -> &Segment<'h> {
        &self.segment
    }

    /// The question a guard is asking, if one is: what the dialog the root
    /// puts up says.
    pub fn question(&self) -> Option<String> {
        self.routed_router().pending().map(|ask| ask.text.to_string())
    }

    /// Answers the guard's question the way the dialog's buttons do: `true`
    /// for the one that lets the navigation go on.
    pub fn answer(&mut self, confirm: bool) -> Act<'h> {
        assert!(
            self.question().is_some(),
            "no guard is asking anything:\n{:#?}",
            self.tree()
        );
        let (dialog, open) = self
            .host
            .adapter()
            .content_dialog(self.page_root())
            .expect("a route tree mounted with `routed` keeps a dialog for a guard's question");
        assert!(open, "a guard is asking, but its dialog is not shown");

        let result = match confirm {
            true => ContentDialogResult::Primary,
            false => ContentDialogResult::None,
        };

        let harness = self.harness;
        let act = harness.record("answer", || {
            let answered = self
                .host
                .test_adapter_mut()
                .complete_content_dialog(dialog, result);
            assert!(answered, "the dialog was not waiting for an answer");
            self.turn();
        });
        self.settle();
        act
    }

    fn routed_router(&self) -> &Rc<Router<WinUi>> {
        self.router.as_ref().unwrap_or_else(|| {
            panic!(
                "{} is not a route tree mounted with `Mounted::routed`",
                std::any::type_name::<S>()
            )
        })
    }

    /// Every route `use_navigate::<R>()` was asked to go to, oldest first.
    /// Panics unless it was mounted [at a route](Self::mount_at) of type `R`.
    pub fn navigated<R: Clone + 'static>(&self) -> Vec<R> {
        self.navigated
            .as_ref()
            .and_then(|navigated| navigated.downcast_ref::<Rc<RefCell<Vec<R>>>>())
            .unwrap_or_else(|| {
                panic!(
                    "{} was not mounted at a {} - use `Mounted::mount_at`",
                    std::any::type_name::<S>(),
                    std::any::type_name::<R>()
                )
            })
            .borrow()
            .clone()
    }

    /// Hands the page or layout one of its own messages, as a widget's
    /// callback would.
    pub fn send<K>(&mut self, message: <S as Mountable<K>>::Message)
    where
        S: Mountable<K>,
    {
        let delivered = sender::<S, <S as Mountable<K>>::Message>()(Signal::Node(message));
        assert!(delivered, "{} no longer takes messages", std::any::type_name::<S>());

        self.turn();
    }

    /// Clicks what carries `mark` the way a pointer would - see
    /// [`click_at`](Self::click_at) for the route it takes - and hands back
    /// what the click set off, as an action named after the mark.
    pub fn click(&mut self, mark: impl Mark) -> Act<'h> {
        let root = self.page_root();
        let found = self.marked(root, &mark);
        self.click_at(found, mark.name(), mark.name())
    }

    /// [`click`](Self::click) for what shows `text` and carries no mark.
    pub fn click_text(&mut self, text: &str) -> Act<'h> {
        let root = self.page_root();
        let found = self.showing(root, text);
        self.click_at(found, text, "click")
    }

    /// Drags what carries `mark` the way a pointer would, and hands back what
    /// the drag set off, as an action named after the mark.
    ///
    /// The button goes down on the element and on everything above it that
    /// listens. The first of those that captures the pointer on press takes
    /// the moves and the release from then on, bubbling up from it; with no
    /// capture, all of them hear everything. As in a window, the capture is
    /// lost just *before* the release arrives, so a drag that cancels itself
    /// on a lost capture never drops. See [`Drag`] for the coordinates.
    pub fn drag(&mut self, mark: impl Mark, drag: Drag) -> Act<'h> {
        let root = self.page_root();
        let found = self.marked(root, &mark);
        self.drag_at(found, drag, mark.name(), mark.name())
    }

    /// Lays the element marked `mark` out at `width` by `height`, at a scale
    /// of 1. The harness has no layout of its own: without this, a
    /// composition host - what a chart or any painted view draws on - never
    /// hears a size, and a pointer over it maps to nothing.
    ///
    /// Every host at or under the element hears it as
    /// `CompositionHostEvent::Metrics`, at once and again whenever it is
    /// observed anew - a redraw that binds a new element keeps the size. A
    /// later size for the same mark replaces this one.
    pub fn size(&mut self, mark: impl Mark, width: f64, height: f64) {
        let root = self.page_root();
        self.marked(root, &mark);

        let name = mark.name();
        self.sizes.retain(|(sized, ..)| *sized != name);
        self.sizes.push((name, width, height));

        self.settle();
    }

    /// The first element, depth first, that carries `mark`.
    pub fn find(&self, mark: impl Mark) -> Option<NodeId> {
        self.first_marked(self.root()?, mark.name())
    }

    /// The first element, depth first, that shows `text`.
    pub fn find_text(&self, text: &str) -> Option<NodeId> {
        self.first_showing(self.root()?, text)
    }

    /// The part of the page that carries `mark`, to find and click in.
    pub fn within(&mut self, mark: impl Mark) -> Within<'_, 'h, S> {
        let root = self.page_root();
        let found = self.marked(root, &mark);

        Within {
            mounted: self,
            root: found,
        }
    }

    /// Item `index` of the first list on the page, brought into view the way
    /// scrolling to it would - a list builds only the items on screen, and
    /// with no screen, none until asked.
    pub fn item(&mut self, index: usize) -> Within<'_, 'h, S> {
        let root = self.page_root();
        let found = self.realize(root, index);

        Within {
            mounted: self,
            root: found,
        }
    }

    /// How many items the first list on the page holds - built or not.
    pub fn item_count(&mut self) -> usize {
        let root = self.page_root();
        self.count(root)
    }

    /// The first item of the first list for which `test` holds, bringing
    /// items into view in order until one does.
    pub fn item_where(&mut self, test: impl Fn(&Node) -> bool) -> Within<'_, 'h, S> {
        let root = self.page_root();
        let found = self.first_item(root, test);

        Within {
            mounted: self,
            root: found,
        }
    }

    /// The first item of the first list that shows `text` somewhere in it.
    pub fn item_with_text(&mut self, text: &str) -> Within<'_, 'h, S> {
        self.item_where(|item| item.find_text(text).is_some())
    }

    /// Every item of the first list, each brought into view.
    pub fn items(&mut self) -> Vec<Node> {
        let root = self.page_root();
        self.all_items(root)
    }

    /// What `node` has for `property`, if it was ever set.
    pub fn property(&self, node: NodeId, property: PropertyId) -> Option<&PropertyValue> {
        self.graph()
            .properties(node)?
            .iter()
            .find(|set| set.id == property)
            .map(|set| &set.value)
    }

    fn graph(&self) -> &RetainedGraph {
        self.host.runtime().graph()
    }

    /// What `node` calls when `event` happens, if it listens for it.
    fn listener(&self, node: NodeId, event: EventId) -> Option<EventValue> {
        self.graph()
            .events(node)?
            .iter()
            .find(|listening| listening.id == event)
            .map(|listening| listening.value.clone())
    }

    /// Raises `event` on `node` the way the control would, if it listens.
    fn raise(&mut self, node: NodeId, event: EventId, payload: EventPayload) {
        if let Some(listener) = self.listener(node, event) {
            self.host
                .queue_event(EventDispatch::new(node, event, listener, payload));
        }
    }

    /// The part of the page from `node` down - one found in a [`Node`], say,
    /// to click or read without a mark of its own.
    pub fn at(&mut self, node: NodeId) -> Within<'_, 'h, S> {
        Within {
            mounted: self,
            root: node,
        }
    }

    fn list_of(&self, under: NodeId) -> NodeId {
        self.list(under)
            .unwrap_or_else(|| panic!("there is no list here:\n{:#?}", self.node(under)))
    }

    fn count(&mut self, under: NodeId) -> usize {
        let list = self.list_of(under);
        self.note_counts();
        self.counts.get(&list).copied().unwrap_or(0)
    }

    /// Reads what the lists said of their length since the last look.
    fn note_counts(&mut self) {
        let batches = self.host.adapter().batches();
        let said: Vec<(NodeId, usize)> = batches[self.read..]
            .iter()
            .flatten()
            .filter_map(|mutation| match mutation {
                Mutation::SetVirtualSource {
                    object, item_count, ..
                } => Some((*object, *item_count)),
                _ => None,
            })
            .collect();
        self.read = batches.len();
        self.counts.extend(said);
    }

    fn first_item(&mut self, under: NodeId, test: impl Fn(&Node) -> bool) -> NodeId {
        let count = self.count(under);

        for index in 0..count {
            let item = self.realize(under, index);
            if test(&self.node(item)) {
                return item;
            }
        }

        panic!("none of the {count} items matches:\n{:#?}", self.all_items(under))
    }

    fn all_items(&mut self, under: NodeId) -> Vec<Node> {
        let count = self.count(under);

        (0..count)
            .map(|index| {
                let item = self.realize(under, index);
                self.node(item)
            })
            .collect()
    }

    fn marked(&self, under: NodeId, mark: &impl Mark) -> NodeId {
        let name = mark.name();
        self.first_marked(under, name)
            .unwrap_or_else(|| panic!("nothing here is marked {name:?}:\n{:#?}", self.node(under)))
    }

    fn showing(&self, under: NodeId, text: &str) -> NodeId {
        self.first_showing(under, text)
            .unwrap_or_else(|| panic!("nothing here shows {text:?}:\n{:#?}", self.node(under)))
    }

    /// The list at or under `under`, with item `index` realized in it.
    fn realize(&mut self, under: NodeId, index: usize) -> NodeId {
        let list = self.list_of(under);

        if let Some(item) = self.realized.get(&(list, index))
            && self.graph().kind(*item).is_some()
        {
            return *item;
        }

        let source_revision = self
            .graph()
            .virtual_source_revision(list)
            .expect("a list has a source");
        let from = self.host.adapter().batches().len();
        self.host.queue_realization(RealizationRequest::Realize {
            collection: list,
            container: RealizedContainer(index as u64 + 1),
            index,
            source_revision,
        });
        self.settle();

        let item = self.host.adapter().batches()[from..]
            .iter()
            .flatten()
            .find_map(|mutation| match mutation {
                Mutation::Realize {
                    parent,
                    index: at,
                    child,
                    ..
                } if *parent == list && *at == index => Some(*child),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the list has no item {index}"));
        self.realized.insert((list, index), item);

        item
    }

    fn list(&self, under: NodeId) -> Option<NodeId> {
        let mut unseen = vec![under];

        while let Some(node) = unseen.pop() {
            if self.graph().virtual_source_revision(node).is_some() {
                return Some(node);
            }

            unseen.extend(self.below(node).into_iter().rev());
        }

        None
    }

    /// What `node` holds, in order: what sits in each place it holds a child,
    /// items brought into view among them, then its flyout's content -
    /// closed or open, as an `Expander`'s content is walked folded or not.
    fn below(&self, node: NodeId) -> Vec<NodeId> {
        let graph = self.graph();
        let adapter = self.host.adapter();
        if graph.kind(node).is_none() {
            return Vec::new();
        }

        let mut below = Vec::new();
        for relation in RELATIONS {
            below.extend(graph.child(node, *relation));
            if let Some(children) = adapter.children(node, *relation) {
                below.extend_from_slice(children);
            }
        }
        below.extend(adapter.flyout(node).map(|(content, _)| content));

        below
    }

    fn kind(&self, node: NodeId) -> Option<ObjectType> {
        self.graph().kind(node)
    }

    fn kind_name(&self, node: NodeId) -> String {
        self.kind(node)
            .map(|kind| format!("{kind:?}"))
            .unwrap_or_else(|| "?".to_string())
    }

    /// If `node` is a control a click turns over, the event that says so, the
    /// state it turns and what it turns it to: the opposite of what it shows
    /// now, or on for one that does not turn back.
    fn flip(&self, node: NodeId) -> Option<(NodeId, EventId, PropertyId, bool)> {
        let kind = self.kind(node)?;
        let (_, event, state, turns_back) = FLIPS.iter().find(|(flips, ..)| *flips == kind)?;

        let now = bool_of(self.property(node, *state)) == Some(true);

        Some((node, *event, *state, !(*turns_back && now)))
    }

    /// Selects `item` in the `NavigationView` it sits in, the way clicking it
    /// would: the view hears its selection changed to the item's tag.
    fn select(&mut self, item: NodeId, parents: &HashMap<NodeId, NodeId>) {
        let mut above = parents.get(&item).copied();
        while let Some(node) = above {
            if self.kind(node) == Some(ObjectType::NavigationView) {
                break;
            }
            above = parents.get(&node).copied();
        }

        let view = above.unwrap_or_else(|| {
            panic!("a NavigationViewItem outside a NavigationView:\n{:#?}", self.node(item))
        });

        let tag = text_of(self.property(item, PropertyId::Tag));
        self.raise(
            view,
            EventId::SelectionChanged,
            EventPayload::Selection(SelectionChange {
                item: Some(item),
                value: tag.map(Into::into),
            }),
        );
    }

    /// A click as WinUI routes one: the pointer bubbles up from `found`
    /// through every element listening for it, and stops at a button, which
    /// takes the pointer for its own click, at a `NavigationViewItem`, which
    /// selects itself, or at a switch, a check box, a toggle button or a
    /// radio button, which turns over - the switch to the opposite of what it
    /// shows, the radio button only ever on. Nothing inside a disabled
    /// control takes it at all.
    fn click_at(&mut self, found: NodeId, label: &str, name: &'static str) -> Act<'h> {
        let parents = self.parents();
        self.refuse_disabled(found, &parents, label, "clicked");

        let mut bubbled = Vec::new();
        let mut button = None;
        let mut item = None;
        let mut flipped = None;
        let mut at = Some(found);

        while let Some(node) = at {
            if let Some(flip) = self.flip(node) {
                flipped = Some(flip);
                break;
            }
            if self.listener(node, EventId::Click).is_some() {
                button = Some(node);
                break;
            }
            if self.kind(node) == Some(ObjectType::NavigationViewItem) {
                item = Some(node);
                break;
            }
            if self.listener(node, EventId::PointerReleased).is_some() {
                bubbled.push(node);
            }
            at = parents.get(&node).copied();
        }

        assert!(
            button.is_some() || item.is_some() || flipped.is_some() || !bubbled.is_empty(),
            "{label:?} is on the page, but nothing at or above it listens for a click"
        );

        let pressed = PointerEventInfo {
            is_left_button_pressed: true,
            ..Default::default()
        };
        for node in &bubbled {
            self.pointer(*node, EventId::PointerPressed, pressed);
        }
        for node in &bubbled {
            self.pointer(*node, EventId::PointerReleased, PointerEventInfo::default());
        }
        if let Some(item) = item {
            self.select(item, &parents);
        }
        if let Some((control, event, state, to)) = flipped {
            self.turn_over(control, event, state, to);
        }
        if let Some(button) = button {
            self.raise(button, EventId::Click, EventPayload::Unit);
        }

        let harness = self.harness;
        harness.record(name, || {
            self.turn();
        })
    }

    /// A drag as WinUI routes one. The pointer goes down with the left button
    /// on `found` and everything above it listening; the first of those that
    /// captures the pointer on press takes the moves and the release from
    /// then on, bubbling up from it. Without a capture every one of them
    /// hears all of it. The view draws again after each step, as it would
    /// between frames.
    ///
    /// The capture is lost *before* the release is heard: the reactor
    /// releases the capture on the way into its release handler, and WinUI
    /// raises the loss right there.
    fn drag_at(&mut self, found: NodeId, drag: Drag, label: &str, name: &'static str) -> Act<'h> {
        let parents = self.parents();
        self.refuse_disabled(found, &parents, label, "dragged");

        let mut path = vec![found];
        while let Some(parent) = parents.get(path.last().expect("starts with `found`")) {
            path.push(*parent);
        }

        assert!(
            path.iter()
                .any(|node| self.listener(*node, EventId::PointerPressed).is_some()),
            "{label:?} is on the page, but nothing at or above it listens for the pointer going down"
        );

        let captured = path.iter().position(|node| {
            bool_of(self.property(*node, PropertyId::CapturePointerOnPress)) == Some(true)
        });
        let held = path[captured.unwrap_or(0)..].to_vec();
        let capture = captured.map(|at| path[at]);

        let harness = self.harness;
        harness.record(name, || {
            for node in &path {
                let pressed = drag.pressed(Some(*node) == capture);
                self.pointer(*node, EventId::PointerPressed, pressed);
            }
            self.turn();

            for step in 1..=drag.steps {
                for node in &held {
                    self.pointer(*node, EventId::PointerMoved, drag.moved(step));
                }
                self.turn();
            }

            if let Some(capture) = capture {
                self.pointer(capture, EventId::PointerCaptureLost, drag.released());
            }
            if !drag.lost {
                for node in &held {
                    self.pointer(*node, EventId::PointerReleased, drag.released());
                }
            }
            self.turn();
        })
    }

    /// Panics if `found` is inside a disabled control, which takes no input.
    fn refuse_disabled(
        &self,
        found: NodeId,
        parents: &HashMap<NodeId, NodeId>,
        label: &str,
        done: &str,
    ) {
        let mut above = Some(found);
        while let Some(node) = above {
            if self.disabled(node) {
                panic!(
                    "{label:?} cannot be {done}: it is inside a disabled {}\n{:#?}",
                    self.node(node).kind,
                    self.node(node)
                );
            }
            above = parents.get(&node).copied();
        }
    }

    /// Whether `node` is a control set to disabled - which takes no input, and
    /// neither does anything inside it.
    fn disabled(&self, node: NodeId) -> bool {
        bool_of(self.property(node, PropertyId::IsEnabled)) == Some(false)
    }

    /// The pointer `event` on `node`, carried the way the element listens
    /// for it: with the pointer's state, or with nothing.
    fn pointer(&mut self, node: NodeId, event: EventId, info: PointerEventInfo) {
        let payload = match self.listener(node, event) {
            Some(EventValue::Unit(_)) => EventPayload::Unit,
            _ => EventPayload::PointerEventInfo(info),
        };
        self.raise(node, event, payload);
    }

    /// Turns `control` over to `to` the way WinUI does: the state changes on
    /// the control, and the event that says so comes with it.
    fn turn_over(&mut self, control: NodeId, event: EventId, state: PropertyId, to: bool) {
        let value = match self.property(control, state) {
            Some(PropertyValue::OptionalBool(_)) => PropertyValue::OptionalBool(Some(to)),
            _ => PropertyValue::Bool(to),
        };
        let observation = Observation::SetProperty {
            object: control,
            property: Property { id: state, value },
        };

        let dispatch = self.listener(control, event).map(|listener| {
            let payload = match listener {
                EventValue::OptionalBool(_) => EventPayload::OptionalBool(Some(to)),
                _ => EventPayload::Bool(to),
            };
            EventDispatch::new(control, event, listener, payload)
        });

        self.host
            .test_adapter_mut()
            .queue_native_event(Some(observation), dispatch);
    }

    /// Runs everything until nothing is left: the harness's work, then what
    /// reached the page, then the page drawing again - as often as one sets
    /// off another.
    pub fn settle(&mut self) {
        loop {
            let ran = self.harness.settled() + self.turn();
            if ran == 0 {
                return;
            }
        }
    }

    /// The outermost element the page drew: what the border it is shown in
    /// holds.
    fn root(&self) -> Option<NodeId> {
        let graph = self.graph();
        let shown = *graph.children(graph.root()?, RelationId::Children)?.first()?;
        graph.child(shown, RelationId::Content)
    }

    fn page_root(&self) -> NodeId {
        self.root().expect("a mounted page has drawn something")
    }

    /// What the page drew, from its outermost element down.
    pub fn tree(&self) -> Node {
        self.node(self.page_root())
    }

    fn first(&self, under: NodeId, test: impl Fn(NodeId) -> bool) -> Option<NodeId> {
        let mut unseen = vec![under];

        while let Some(node) = unseen.pop() {
            if test(node) {
                return Some(node);
            }

            unseen.extend(self.below(node).into_iter().rev());
        }

        None
    }

    fn first_marked(&self, under: NodeId, name: &str) -> Option<NodeId> {
        self.first(under, |node| self.mark_of(node).as_deref() == Some(name))
    }

    fn first_showing(&self, under: NodeId, text: &str) -> Option<NodeId> {
        self.first(under, |node| self.text(node).as_deref() == Some(text))
    }

    /// What a text block shows - a text box's text is what was typed into
    /// it, not something on the page to find.
    fn text(&self, node: NodeId) -> Option<String> {
        match self.kind(node)? {
            ObjectType::TextBlock => text_of(self.property(node, PropertyId::Text)),
            _ => None,
        }
    }

    fn mark_of(&self, node: NodeId) -> Option<String> {
        text_of(self.property(node, PropertyId::AutomationId))
    }

    fn node(&self, id: NodeId) -> Node {
        Node {
            at: id,
            kind: self.kind_name(id),
            text: self.text(id),
            id: self.mark_of(id),
            children: self
                .below(id)
                .into_iter()
                .map(|child| self.node(child))
                .collect(),
        }
    }

    fn parents(&self) -> HashMap<NodeId, NodeId> {
        let mut parents = HashMap::new();
        let mut unseen: Vec<NodeId> = self.root().into_iter().collect();

        while let Some(node) = unseen.pop() {
            for child in self.below(node) {
                parents.insert(child, node);
                unseen.push(child);
            }
        }

        parents
    }

    /// One pass of what the window would do between frames: deliver the
    /// events waiting for the page, then let its components update and draw.
    fn turn(&mut self) -> usize {
        let drained = self
            .host
            .drain(TURNS)
            .unwrap_or_else(|error| panic!("running the page's components: {error:?}"));

        drained.dispatched + drained.dropped + self.measure()
    }

    /// Tells each composition host under a [sized](Self::size) element the
    /// size it is at - once per size, and again when it is observed anew.
    fn measure(&mut self) -> usize {
        if self.sizes.is_empty() {
            return 0;
        }
        let Some(root) = self.root() else {
            return 0;
        };

        let mut hosts: Vec<((NodeId, u64), Callback<CompositionHostEvent>)> = Vec::new();
        for request in self.host.adapter().imperatives() {
            match request {
                ImperativeRequest::ObserveCompositionHost {
                    object,
                    observation,
                    callback,
                } => hosts.push(((*object, *observation), callback.clone())),
                ImperativeRequest::RevokeObservation {
                    object,
                    observation,
                } => hosts.retain(|(observed, _)| *observed != (*object, *observation)),
                _ => {}
            }
        }

        let mut sized = HashMap::new();
        for (name, width, height) in &self.sizes {
            let Some(element) = self.first_marked(root, name) else {
                continue;
            };

            let mut unseen = vec![element];
            while let Some(node) = unseen.pop() {
                sized.insert(node, (*width, *height));
                unseen.extend(self.below(node));
            }
        }

        let mut told = 0;
        for (observed, callback) in hosts {
            let Some(&(width, height)) = sized.get(&observed.0) else {
                continue;
            };
            if self.told.insert(observed, (width, height)) == Some((width, height)) {
                continue;
            }

            callback.call(CompositionHostEvent::Metrics {
                width,
                height,
                scale: 1.0,
            });
            told += 1;
        }

        told
    }
}

impl Mounted<'_, ()> {
    /// Every route of `R`'s tree, each mounted on its own, drawn and settled.
    ///
    /// What a read that reaches nothing, or a fill nobody placed, would first
    /// show a user, all of it at once: the error names every route that
    /// failed and what it said. A route whose fields have no `Default` cannot
    /// be made here, and is named as not mounted - mount it with
    /// [`routed`](Mounted::routed) and the value it needs.
    ///
    /// A read in a branch the first draw does not take is not covered.
    pub fn each_route<R>(harness: &Harness) -> anyhow::Result<()>
    where
        R: RouteChain<WinUi> + Clone + PartialEq + 'static,
    {
        let mut failed = Vec::new();

        for sample in R::samples() {
            let route = match sample {
                Ok(route) => route,
                Err(name) => {
                    failed.push(format!(
                        "`{name}` was not mounted: a field of its route has no `Default`"
                    ));
                    continue;
                }
            };

            let name = route.name();
            let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Mounted::routed(harness, route).map(|mut mounted| mounted.settle())
            }));
            match drawn {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failed.push(format!("`{name}`: {error:#}")),
                Err(panic) => failed.push(format!("`{name}`: {}", said(&*panic))),
            }
        }

        match failed.is_empty() {
            true => Ok(()),
            false => Err(anyhow::anyhow!(
                "{} of the routes of `{}` failed:\n{}",
                failed.len(),
                guinea_router::observability::short(std::any::type_name::<R>()),
                failed.join("\n")
            )),
        }
    }
}

/// What a panic said, when it said it in text.
fn said(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .unwrap_or_else(|| "a panic that said nothing in text".to_string())
}

impl<'h, R> Mounted<'h, R>
where
    R: RouteChain<WinUi> + Clone + PartialEq + 'static,
{
    /// The whole route tree at `initial`, mounted the way a window mounts it:
    /// a [`RouterRoot`] with a router of its own.
    ///
    /// Navigation happens here - through [`navigate`](Self::navigate), or a
    /// click that ends in `use_navigate` - and moves `use_route`. Segments are
    /// installed and torn down as in the application: the layouts and pages
    /// left behind lose their scopes, actors and timers, and the new ones
    /// install, start and draw under the outlet above them. What the harness
    /// provides reaches every segment, whenever it is installed. A guard's
    /// question goes up as the root's dialog - see [`answer`](Self::answer).
    ///
    /// ```ignore
    /// let mut app = Mounted::routed(&h, Route::Processes {})?;
    /// app.navigate(Route::Services {});
    /// app.navigate(Route::Processes {});
    /// assert!(app.find(ProcessesMark::Loading).is_none());
    /// ```
    pub fn routed(harness: &'h Harness, initial: R) -> anyhow::Result<Self> {
        let rooted = Rooted {
            app: harness.application(),
            initial,
        };

        let built = host(component::<Shown>(
            ROOT,
            View::component::<RouterRoot<R>>(rooted),
        ));

        let host = built.map_err(|refused| {
            anyhow::anyhow!("mounting the {} tree: {refused}", std::any::type_name::<R>())
        })?;

        let router = ROUTER
            .with(|kept| kept.borrow_mut().take())
            .expect("the root builds its router as it is created");

        let mut mounted = Self {
            harness,
            host,
            read: 0,
            realized: HashMap::new(),
            counts: HashMap::new(),
            navigated: None,
            router: Some(router),
            sizes: Vec::new(),
            told: HashMap::new(),
            kind: PhantomData,
            segment: harness.segment(),
        };
        mounted.settle();

        Ok(mounted)
    }

    /// Goes to `to` the way `use_navigate` does - guards, teardown and all -
    /// and hands back what the navigation set off.
    pub fn navigate(&mut self, to: R) -> Act<'h> {
        let nav = NAV
            .with(|kept| {
                kept.borrow()
                    .as_ref()
                    .and_then(|nav| nav.downcast_ref::<NavigateHandle<WinUi, R>>())
                    .cloned()
            })
            .expect("the root hands its tree a way to navigate as it draws");

        let harness = self.harness;
        let act = harness.record("navigate", || {
            nav.to(to);
            self.turn();
        });
        self.settle();
        act
    }

    /// Where the application is.
    pub fn route(&self) -> R {
        self.routed_router()
            .current_route::<R>()
            .expect("the first route installed")
    }

    /// The layouts and pages mounted now, outermost first, by type name.
    pub fn segments(&self) -> Vec<&'static str> {
        self.routed_router()
            .active_chain()
            .unwrap_or_default()
            .iter()
            .map(|entry| (entry.type_name)())
            .collect()
    }

    /// Whether the layout or page `T` is mounted now.
    pub fn is_mounted<T: 'static>(&self) -> bool {
        self.segments().contains(&std::any::type_name::<T>())
    }

    /// `S` as the mounted segment that claimed it holds it: a layout, a page
    /// or a part, wherever it sits. What no mounted segment claims is read
    /// the way [`Harness::state`] reads it.
    ///
    /// Panics when two mounted segments claim `S`: which one is meant is the
    /// test's to say, through [`Harness::over`].
    pub fn state<St: Reducer>(&self) -> Rc<St> {
        self.owning::<St>().state::<St>()
    }

    /// What a page reading `S` is handed to act with, from the mounted
    /// segment that claimed it. See [`state`](Self::state).
    pub fn dispatch<St: Reducer>(&self) -> Dispatch {
        self.owning::<St>().dispatch::<St>()
    }

    /// Acts on the feature that owns `S` in the mounted tree, and hands back
    /// what the action set off. See [`state`](Self::state).
    pub fn act<St: Reducer>(&self, action: impl Sized + 'static) -> Act<'h> {
        self.owning::<St>().act::<St>(action)
    }

    /// The segment of the mounted tree that claimed `S`, or the harness's own.
    fn owning<St: Reducer>(&self) -> Segment<'h> {
        let mut mounted: Vec<Scope> = Vec::new();
        for scope in self.routed_router().active_scopes().iter().flat_map(|scopes| scopes.iter()) {
            mounted.push(*scope);
            mounted.extend(scope.beside().iter().flat_map(Scope::subtree));
        }

        let claiming: Vec<Scope> = mounted.into_iter().filter(Scope::claims::<St>).collect();
        match claiming.as_slice() {
            [] => self.harness.segment(),
            [one] => self.harness.over(*one),
            many => panic!(
                "{} is claimed by {} mounted segments - say which one with `Harness::over`",
                std::any::type_name::<St>(),
                many.len()
            ),
        }
    }
}

/// Part of a mounted page: what is found and clicked through it is found
/// under it, so the same mark in every row of a list names one thing again.
pub struct Within<'m, 'h, S> {
    mounted: &'m mut Mounted<'h, S>,
    root: NodeId,
}

impl<'h, S: 'static> Within<'_, 'h, S> {
    /// The part of this part that carries `mark`.
    pub fn within(self, mark: impl Mark) -> Self {
        let root = self.mounted.marked(self.root, &mark);
        Self { root, ..self }
    }

    /// Item `index` of the first list in this part - see [`Mounted::item`].
    pub fn item(self, index: usize) -> Self {
        let root = self.mounted.realize(self.root, index);
        Self { root, ..self }
    }

    /// See [`Mounted::item_count`].
    pub fn item_count(&mut self) -> usize {
        self.mounted.count(self.root)
    }

    /// See [`Mounted::item_where`].
    pub fn item_where(self, test: impl Fn(&Node) -> bool) -> Self {
        let root = self.mounted.first_item(self.root, test);
        Self { root, ..self }
    }

    /// See [`Mounted::item_with_text`].
    pub fn item_with_text(self, text: &str) -> Self {
        self.item_where(|item| item.find_text(text).is_some())
    }

    /// See [`Mounted::items`].
    pub fn items(&mut self) -> Vec<Node> {
        self.mounted.all_items(self.root)
    }

    /// What this part's outermost element has for `property`.
    pub fn property(&self, property: PropertyId) -> Option<&PropertyValue> {
        self.mounted.property(self.root, property)
    }

    pub fn find(&self, mark: impl Mark) -> Option<NodeId> {
        self.mounted.first_marked(self.root, mark.name())
    }

    pub fn find_text(&self, text: &str) -> Option<NodeId> {
        self.mounted.first_showing(self.root, text)
    }

    /// Clicks what carries `mark` in this part - see [`Mounted::click`].
    pub fn click(self, mark: impl Mark) -> Act<'h> {
        let found = self.mounted.marked(self.root, &mark);
        self.mounted.click_at(found, mark.name(), mark.name())
    }

    pub fn click_text(self, text: &str) -> Act<'h> {
        let found = self.mounted.showing(self.root, text);
        self.mounted.click_at(found, text, "click")
    }

    /// Clicks this part itself: a row, to select it.
    pub fn click_here(self) -> Act<'h> {
        self.mounted.click_at(self.root, "this part", "click")
    }

    /// Drags what carries `mark` in this part - see [`Mounted::drag`].
    pub fn drag(self, mark: impl Mark, drag: Drag) -> Act<'h> {
        let found = self.mounted.marked(self.root, &mark);
        self.mounted.drag_at(found, drag, mark.name(), mark.name())
    }

    /// Drags this part itself.
    pub fn drag_here(self, drag: Drag) -> Act<'h> {
        self.mounted.drag_at(self.root, drag, "this part", "drag")
    }

    /// What this part drew, from its outermost element down.
    pub fn tree(&self) -> Node {
        self.mounted.node(self.root)
    }
}

fn text_of(value: Option<&PropertyValue>) -> Option<String> {
    match value? {
        PropertyValue::String(text) => Some(text.to_string()),
        _ => None,
    }
}

fn bool_of(value: Option<&PropertyValue>) -> Option<bool> {
    match value? {
        PropertyValue::Bool(on) => Some(*on),
        PropertyValue::OptionalBool(on) => *on,
        _ => None,
    }
}
