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
use guinea_core::mark::Mark;
use guinea_core::scope::Scope;
use guinea_router::router::{
    Mount, NavigateHandle, RouteChain, RouteSink, SegmentEntry, SegmentProps,
};
use windows_reactor::test::{
    Command, EventId, EventPayload, Pump, QueuedEvent, RealizedContainer, RecordingRuntime,
    SelectionChange, SlotId,
};
use windows_reactor::{Border, PointerEventInfo, View};

pub use windows_reactor::test::{NodeId, PropertyId, PropertyValue};

use crate::mark::MarkExt;
use crate::winui::{
    Layout, LayoutNode, Page, PageNode, Signal, WinUi, install_layout, install_page,
    layout_entry, nav_context, route_context, segment_entry,
};

/// How many component turns one pass may run before it looks again.
const TURNS: usize = 64;

/// Every control's `IsEnabled`: the reactor names it per control.
const ENABLED: &[PropertyId] = &[
    PropertyId::AppBarButtonIsEnabled,
    PropertyId::AutoSuggestBoxIsEnabled,
    PropertyId::ButtonIsEnabled,
    PropertyId::CalendarDatePickerIsEnabled,
    PropertyId::CalendarViewIsEnabled,
    PropertyId::CheckBoxIsEnabled,
    PropertyId::ColorPickerIsEnabled,
    PropertyId::ComboBoxIsEnabled,
    PropertyId::DatePickerIsEnabled,
    PropertyId::DropDownButtonIsEnabled,
    PropertyId::HyperlinkButtonIsEnabled,
    PropertyId::ListBoxIsEnabled,
    PropertyId::NavigationViewIsEnabled,
    PropertyId::NumberBoxIsEnabled,
    PropertyId::PasswordBoxIsEnabled,
    PropertyId::ProgressBarIsEnabled,
    PropertyId::ProgressRingIsEnabled,
    PropertyId::RadioButtonIsEnabled,
    PropertyId::RepeatButtonIsEnabled,
    PropertyId::RichEditBoxIsEnabled,
    PropertyId::SliderIsEnabled,
    PropertyId::SplitButtonIsEnabled,
    PropertyId::TextBoxIsEnabled,
    PropertyId::TimePickerIsEnabled,
    PropertyId::ToggleButtonIsEnabled,
    PropertyId::ToggleSwitchIsEnabled,
];

/// Every named place a control holds a child besides its plain children - a
/// `NavigationView`'s menu items and content, an `Expander`'s header. The
/// reactor keeps its own list of which control has which private, so the
/// tree is walked through all of them - a `NavigationView`'s pane before its
/// content, the way it reads on screen.
const SLOTS: &[SlotId] = &[
    SlotId::TextBoxHeader,
    SlotId::AutoSuggestBoxHeader,
    SlotId::PasswordBoxHeader,
    SlotId::NumberBoxHeader,
    SlotId::SliderHeader,
    SlotId::TitleBarContent,
    SlotId::TitleBarRightHeader,
    SlotId::NavigationViewHeader,
    SlotId::NavigationViewPaneCustomContent,
    SlotId::NavigationViewMenuItems,
    SlotId::NavigationViewFooterMenuItems,
    SlotId::NavigationViewPaneFooter,
    SlotId::NavigationViewContent,
    SlotId::NavigationViewItemIcon,
    SlotId::NavigationViewItemContent,
    SlotId::NavigationViewItemMenuItems,
    SlotId::SplitViewPane,
    SlotId::SplitViewContent,
    SlotId::ToggleSwitchHeader,
    SlotId::ToggleSwitchOnContent,
    SlotId::ToggleSwitchOffContent,
    SlotId::RadioButtonsHeader,
    SlotId::ListBoxItems,
    SlotId::ExpanderHeader,
    SlotId::ExpanderContent,
    SlotId::ComboBoxHeader,
    SlotId::PivotItems,
    SlotId::FlipViewItems,
    SlotId::SelectorBarItems,
    SlotId::SelectorBarItemIcon,
    SlotId::TabViewTabItems,
    SlotId::CommandBarPrimaryCommands,
    SlotId::CommandBarSecondaryCommands,
    SlotId::AppBarButtonIcon,
    SlotId::MenuBarItems,
    SlotId::DatePickerHeader,
    SlotId::TimePickerHeader,
    SlotId::CalendarDatePickerHeader,
    SlotId::ListViewItems,
    SlotId::GridViewItems,
    SlotId::RichEditBoxHeader,
    SlotId::ViewboxChild,
];

/// The controls a click turns over, by kind: the event that says so, the
/// state it turns, and whether a click turns it back. A radio button does
/// not: a click only ever checks it.
const FLIPS: &[(&str, EventId, PropertyId, bool)] = &[
    (
        "ToggleSwitch",
        EventId::ToggleSwitchToggled,
        PropertyId::ToggleSwitchIsOn,
        true,
    ),
    (
        "CheckBox",
        EventId::CheckBoxIsCheckedChanged,
        PropertyId::CheckBoxIsChecked,
        true,
    ),
    (
        "ToggleButton",
        EventId::ToggleButtonIsCheckedChanged,
        PropertyId::ToggleButtonIsChecked,
        true,
    ),
    (
        "RadioButton",
        EventId::RadioButtonChecked,
        PropertyId::RadioButtonIsChecked,
        false,
    ),
];

type Sender<M> = Rc<dyn Fn(Signal<M>) -> bool>;

thread_local! {
    static SENDERS: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
    static CHAINS: RefCell<HashMap<(TypeId, usize), &'static [SegmentEntry<WinUi>]>> =
        RefCell::new(HashMap::new());
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
    SegmentEntry::new::<Outlet>(|_, _| Ok(()), |_, _| true, &MountOutlet, false);

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
            capture_succeeded: captures,
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
    pump: Pump<RecordingRuntime>,
    /// The items brought into view so far, by list and index.
    realized: HashMap<(NodeId, usize), NodeId>,
    /// How many items each list holds, as it last said.
    counts: HashMap<NodeId, usize>,
    /// Where it asked to go, when mounted [at a route](Self::mount_at): an
    /// `Rc<RefCell<Vec<R>>>` for that route type.
    navigated: Option<Box<dyn Any>>,
    segment: PhantomData<S>,
}

impl<'h, S: 'static> Mounted<'h, S> {
    /// Installs `S` into `segment` - what it `Installs`, and the node it
    /// starts as - and mounts it, the way a navigation to it would.
    pub fn mount<K>(
        segment: &Segment<'h>,
        params: <S as Mountable<K>>::Params,
    ) -> anyhow::Result<Self>
    where
        S: Mountable<K>,
    {
        Self::mount_with(segment, params, |view| view)
    }

    /// [`mount`](Self::mount), with the view handed to `wrap` first - for
    /// what a layout above it would give it, such as a context:
    /// `|page| View::provide(&SCHEME, scheme, page)`.
    pub fn mount_with<K>(
        segment: &Segment<'h>,
        params: <S as Mountable<K>>::Params,
        wrap: impl FnOnce(View) -> View,
    ) -> anyhow::Result<Self>
    where
        S: Mountable<K>,
    {
        let cx = segment.context();
        S::install(cx, &params)?;

        let scopes: Vec<Rc<Scope>> = cx
            .ancestors
            .iter()
            .cloned()
            .chain([cx.scope.clone()])
            .collect();
        let depth = scopes.len();

        let props = SegmentProps {
            chain: chain::<S, K>(depth),
            scopes: Rc::new(scopes),
            cursor: depth - 1,
        };

        let mut runtime = RecordingRuntime::default();
        runtime.record_commands(true);

        let mut pump = Pump::new(runtime);
        pump.mount_view(wrap(S::component(props)))
            .map_err(|refused| anyhow::anyhow!("mounting {}: {refused:?}", std::any::type_name::<S>()))?;

        let mut mounted = Self {
            harness: segment.harness(),
            pump,
            realized: HashMap::new(),
            counts: HashMap::new(),
            navigated: None,
            segment: PhantomData,
        };
        mounted.settle();

        Ok(mounted)
    }

    /// [`mount`](Self::mount), with `route` as where the application is: what
    /// `use_route::<R>()` returns. `use_navigate::<R>()` moves nothing - where
    /// it was asked to go is kept for [`navigated`](Self::navigated), and the
    /// route `use_route` returns stays `route`.
    pub fn mount_at<K, R>(
        segment: &Segment<'h>,
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
            let view = View::provide(route_context::<R>(), Some(route), view);
            View::provide(nav_context::<R>(), Some(nav), view)
        })?;
        mounted.navigated = Some(Box::new(navigated));

        Ok(mounted)
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

    /// The first element, depth first, that carries `mark`.
    pub fn find(&self, mark: impl Mark) -> Option<NodeId> {
        self.first(self.root()?, PropertyId::AutomationId, mark.name())
    }

    /// The first element, depth first, that shows `text`.
    pub fn find_text(&self, text: &str) -> Option<NodeId> {
        self.first(self.root()?, PropertyId::TextBlockText, text)
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
        self.pump.runtime().node(node)?.property(property)
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
        let said: Vec<(NodeId, usize)> = self
            .pump
            .runtime()
            .commands()
            .iter()
            .flatten()
            .filter_map(|command| match command {
                Command::CreateVirtualCollection {
                    node, item_count, ..
                }
                | Command::ResetVirtualCollection {
                    node, item_count, ..
                } => Some((*node, *item_count)),
                _ => None,
            })
            .collect();
        self.counts.extend(said);

        let runtime = self.pump.runtime_mut();
        runtime.record_commands(false);
        runtime.record_commands(true);
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
        self.first(under, PropertyId::AutomationId, name)
            .unwrap_or_else(|| panic!("nothing here is marked {name:?}:\n{:#?}", self.node(under)))
    }

    fn showing(&self, under: NodeId, text: &str) -> NodeId {
        self.first(under, PropertyId::TextBlockText, text)
            .unwrap_or_else(|| panic!("nothing here shows {text:?}:\n{:#?}", self.node(under)))
    }

    /// The list at or under `under`, with item `index` realized in it.
    fn realize(&mut self, under: NodeId, index: usize) -> NodeId {
        let list = self.list_of(under);

        let children = |mounted: &Self| -> Vec<NodeId> {
            mounted
                .pump
                .runtime()
                .node(list)
                .map(|recorded| recorded.children().to_vec())
                .unwrap_or_default()
        };

        if let Some(item) = self.realized.get(&(list, index))
            && children(self).contains(item)
        {
            return *item;
        }

        let before = children(self);
        self.pump
            .runtime_mut()
            .queue_realize(list, RealizedContainer(index as u64 + 1), index);
        self.pump
            .process_realizations()
            .expect("bringing an item into view");
        self.settle();

        let item = children(self)
            .into_iter()
            .find(|child| !before.contains(child))
            .unwrap_or_else(|| panic!("the list has no item {index}"));
        self.realized.insert((list, index), item);

        item
    }

    fn list(&self, under: NodeId) -> Option<NodeId> {
        let mut unseen = vec![under];

        while let Some(node) = unseen.pop() {
            if self.pump.runtime().source_revision(node).is_some() {
                return Some(node);
            }

            unseen.extend(self.below(node).into_iter().rev());
        }

        None
    }

    /// What `node` holds, in order: its children, then what sits in each of
    /// its slots, then its flyout's content - closed or open, as an
    /// `Expander`'s content is walked folded or not.
    fn below(&self, node: NodeId) -> Vec<NodeId> {
        let runtime = self.pump.runtime();
        let Some(recorded) = runtime.node(node) else {
            return Vec::new();
        };

        let mut below = recorded.children().to_vec();
        for slot in SLOTS {
            below.extend(recorded.slot(*slot));
            below.extend_from_slice(recorded.slot_children(*slot));
        }
        below.extend(runtime.flyout(node).map(|(content, _)| content));

        below
    }

    fn kind(&self, node: NodeId) -> String {
        self.pump
            .runtime()
            .node(node)
            .and_then(|recorded| recorded.kind())
            .map(|kind| format!("{kind:?}"))
            .unwrap_or_else(|| "?".to_string())
    }

    /// If `node` is a control a click turns over, the event that says so, the
    /// state it turns and what it turns it to: the opposite of what it shows
    /// now, or on for one that does not turn back.
    fn flip(&self, node: NodeId) -> Option<(NodeId, EventId, PropertyId, bool)> {
        let kind = self.kind(node);
        let (_, event, state, turns_back) = FLIPS.iter().find(|(flips, ..)| *flips == kind)?;

        let now = matches!(self.property(node, *state), Some(PropertyValue::Bool(true)));

        Some((node, *event, *state, !(*turns_back && now)))
    }

    /// Selects `item` in the `NavigationView` it sits in, the way clicking it
    /// would: the view hears its selection changed to the item's tag.
    fn select(&mut self, item: NodeId, parents: &HashMap<NodeId, NodeId>) {
        let mut above = parents.get(&item).copied();
        while let Some(node) = above {
            if self.kind(node) == "NavigationView" {
                break;
            }
            above = parents.get(&node).copied();
        }

        let view = above.unwrap_or_else(|| {
            panic!("a NavigationViewItem outside a NavigationView:\n{:#?}", self.node(item))
        });
        let Some(revision) = self
            .pump
            .event_revision(view, EventId::NavigationViewSelectionChanged)
        else {
            return;
        };

        let tag = text_of(self.property(item, PropertyId::NavigationViewItemTag));
        self.pump.queue_event(QueuedEvent::new(
            view,
            EventId::NavigationViewSelectionChanged,
            revision,
            EventPayload::SelectionChange(SelectionChange {
                item: Some(item),
                tag,
            }),
        ));
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
            if self.pump.event_revision(node, EventId::ButtonClick).is_some() {
                button = Some(node);
                break;
            }
            if self.kind(node) == "NavigationViewItem" {
                item = Some(node);
                break;
            }
            if self
                .pump
                .event_revision(node, EventId::BorderPointerReleased)
                .is_some()
            {
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
            self.pointer(*node, EventId::BorderPointerPressed, pressed);
        }
        for node in &bubbled {
            self.pointer(*node, EventId::BorderPointerReleased, PointerEventInfo::default());
        }
        if let Some(item) = item {
            self.select(item, &parents);
        }
        if let Some((control, event, state, to)) = flipped {
            self.pump
                .runtime_mut()
                .record_property_observation(control, state, PropertyValue::Bool(to))
                .expect("the control a click reached is in the tree");

            if let Some(revision) = self.pump.event_revision(control, event) {
                self.pump.queue_event(QueuedEvent::new(
                    control,
                    event,
                    revision,
                    EventPayload::Bool(to),
                ));
            }
        }
        if let Some(button) = button
            && let Some(revision) = self.pump.event_revision(button, EventId::ButtonClick)
        {
            self.pump.queue_event(QueuedEvent::new(
                button,
                EventId::ButtonClick,
                revision,
                EventPayload::Unit,
            ));
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
                .any(|node| self.pump.event_revision(*node, EventId::BorderPointerPressed).is_some()),
            "{label:?} is on the page, but nothing at or above it listens for the pointer going down"
        );

        let captured = path.iter().position(|node| {
            matches!(
                self.property(*node, PropertyId::BorderCapturePointerOnPress),
                Some(PropertyValue::Bool(true))
            )
        });
        let held = path[captured.unwrap_or(0)..].to_vec();
        let capture = captured.map(|at| path[at]);

        let harness = self.harness;
        harness.record(name, || {
            for node in &path {
                let pressed = drag.pressed(Some(*node) == capture);
                self.pointer(*node, EventId::BorderPointerPressed, pressed);
            }
            self.turn();

            for step in 1..=drag.steps {
                for node in &held {
                    self.pointer(*node, EventId::BorderPointerMoved, drag.moved(step));
                }
                self.turn();
            }

            if let Some(capture) = capture
                && let Some(revision) = self
                    .pump
                    .event_revision(capture, EventId::BorderPointerCaptureLost)
            {
                self.pump.queue_event(QueuedEvent::new(
                    capture,
                    EventId::BorderPointerCaptureLost,
                    revision,
                    EventPayload::Unit,
                ));
            }
            if !drag.lost {
                for node in &held {
                    self.pointer(*node, EventId::BorderPointerReleased, drag.released());
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
        ENABLED
            .iter()
            .any(|id| matches!(self.property(node, *id), Some(PropertyValue::Bool(false))))
    }

    fn pointer(&mut self, node: NodeId, event: EventId, info: PointerEventInfo) {
        if let Some(revision) = self.pump.event_revision(node, event) {
            self.pump.queue_event(QueuedEvent::new(
                node,
                event,
                revision,
                EventPayload::PointerEventInfo(info),
            ));
        }
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

    /// The outermost element the page drew: what the window holds.
    fn root(&self) -> Option<NodeId> {
        let window = self.pump.window()?;
        self.pump.runtime().node(window)?.children().first().copied()
    }

    fn page_root(&self) -> NodeId {
        self.root().expect("a mounted page has drawn something")
    }

    /// What the page drew, from its outermost element down.
    pub fn tree(&self) -> Node {
        self.node(self.page_root())
    }

    fn first(&self, under: NodeId, property: PropertyId, value: &str) -> Option<NodeId> {
        let mut unseen = vec![under];

        while let Some(node) = unseen.pop() {
            let recorded = self.pump.runtime().node(node)?;

            if text_of(recorded.property(property)).as_deref() == Some(value) {
                return Some(node);
            }

            unseen.extend(self.below(node).into_iter().rev());
        }

        None
    }

    fn node(&self, id: NodeId) -> Node {
        let Some(recorded) = self.pump.runtime().node(id) else {
            return Node {
                at: id,
                kind: "?".to_string(),
                text: None,
                id: None,
                children: Vec::new(),
            };
        };

        Node {
            at: id,
            kind: recorded
                .kind()
                .map(|kind| format!("{kind:?}"))
                .unwrap_or_else(|| "?".to_string()),
            text: text_of(recorded.property(PropertyId::TextBlockText)),
            id: text_of(recorded.property(PropertyId::AutomationId)),
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
        let events = self.pump.dispatch_events().expect("delivering events to the page");
        let turns = self
            .pump
            .dispatch_components(TURNS)
            .expect("running the page's components");

        events + turns
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
        self.mounted.first(self.root, PropertyId::AutomationId, mark.name())
    }

    pub fn find_text(&self, text: &str) -> Option<NodeId> {
        self.mounted.first(self.root, PropertyId::TextBlockText, text)
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
        PropertyValue::Str(text) => Some(text.to_string()),
        _ => None,
    }
}
