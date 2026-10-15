#![cfg(all(windows, feature = "winui"))]

//! A WinUI page under the harness: mounted with no window, typed into and
//! clicked, and read back as the tree it drew - with the features behind it
//! running in the seed's order.

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::{Mounted, PropertyId, PropertyValue};
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};
use windows_reactor::{
    Border, Button, Callback, CheckBox, CompositionHostEvent, ElementObservation, ElementRef,
    Flyout, FlyoutExt, Grid, ItemsRepeater, PointerEventInfo, RadioButton, StackPanel, TextBlock,
    ToggleSwitch, View, VirtualSource, provide,
};

const CATALOGUE: [&str; 6] = ["guinea", "guinea-app", "gui", "gum", "gulp", "gust"];

#[derive(guinea::Mark)]
enum Marks {
    Search,
    Open,
    Chevron,
    Remove,
    Grip,
    Switch,
    Check,
    Radio,
    Charts,
    ShowDisk,
    Speak,
    Plot,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Results {
    pub query: String,
    pub found: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub query: String,
    pub found: Vec<&'static str>,
}

#[reducer]
fn results(this: &mut Results, update: Found) {
    this.query = update.query;
    this.found = update.found;
}

#[derive(Clone, Debug)]
pub struct Query(pub String);

#[derive(Debug)]
pub struct Searcher {
    pub push: Push<Results>,
}

actor! {
    Searcher {
        handlers { Query => { task Found }, Found }
    }
}

#[handler]
fn query(_this: &mut Searcher, Query(text): Query, cx: Cx) {
    cx.task().spawn(async move {
        guinea::core::executor::random_delay().await;
        let found = CATALOGUE.iter().copied().filter(|name| name.contains(text.as_str())).collect();
        Found { query: text, found }
    });
}

#[handler]
fn found(this: &mut Searcher, found: Found) {
    this.push.send(found);
}

feature! {
    pub Search {
        exports { Results }
    }
}

#[installs]
fn search(cx: &FeatureInitContext) -> anyhow::Result<Search> {
    let (results, _) = cx.state::<Results>().driven_by(|push| Searcher { push });
    Ok(Search(results))
}

/// A box to type into, a button, and what the last search found.
#[derive(Default)]
pub struct SearchPage {
    typed: String,
}

pub enum Typing {
    Typed(String),
}

#[page]
impl Page for SearchPage {
    type Params = ();
    type Installs = Search;
    type Message = Typing;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Self, Search)> {
        Ok((Self::default(), ctx.install(&())?))
    }

    fn update(&mut self, message: Typing, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Typing::Typed(text) => self.typed = text,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (results, dispatch) = cx.read::<Results>();
        let typed = self.typed.clone();

        StackPanel::new().children((
            TextBlock::new().text(format!("typed: {}", self.typed)),
            Button::new()
                .mark(Marks::Search)
                .on_click(move || dispatch.emit(Query(typed.clone())))
                .content(TextBlock::new().text("Search")),
            TextBlock::new().text(format!("found {} for {:?}", results.found.len(), results.query)),
        ))
        .into()
    }
}

#[guinea::test(iterations = 30)]
fn typing_and_clicking_search_draws_what_it_found(h: &mut Harness) {
    let mut page = Mounted::<SearchPage>::mount(h.segment(),()).unwrap();
    assert!(page.find_text("found 0 for \"\"").is_some(), "{:#?}", page.tree());

    page.send(Typing::Typed("guinea".into()));
    let clicked = page.click(Marks::Search);
    clicked.settle();
    assert!(clicked.chain().handled::<Query>(), "{:#?}", clicked.chain());
    assert!(clicked.chain().pushed::<Results>(), "{:#?}", clicked.chain());
    page.settle();

    assert!(page.find_text("typed: guinea").is_some(), "{:#?}", page.tree());
    assert!(page.find_text("found 2 for \"guinea\"").is_some(), "{:#?}", page.tree());
}

#[guinea::test(iterations = 4)]
fn the_tree_a_page_draws_is_data(h: &mut Harness) {
    let mut page = Mounted::<SearchPage>::mount(h.segment(),()).unwrap();
    page.send(Typing::Typed("gu".into()));
    page.click(Marks::Search);
    page.settle();

    insta::assert_ron_snapshot!("search_page", page.tree());
}

/// The whole catalogue as a list, each item with a button that opens it.
#[derive(Default)]
pub struct CataloguePage {
    opened: Option<&'static str>,
}

pub enum Browsing {
    Opened(usize),
}

#[page]
impl Page for CataloguePage {
    type Params = ();
    type Message = Browsing;

    fn update(&mut self, message: Browsing, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Browsing::Opened(at) => self.opened = Some(CATALOGUE[at]),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let open = cx.on(Browsing::Opened);

        let items = VirtualSource::new(
            CATALOGUE.len() as u64,
            CATALOGUE.len(),
            |index| index,
            move |index| {
                let open = open.clone();

                StackPanel::new()
                    .children((
                        TextBlock::new().text(CATALOGUE[index]),
                        Button::new()
                            .mark(Marks::Open)
                            .on_click(move || {
                                let _ = open.call(index);
                            })
                            .content(TextBlock::new().text("Open")),
                    ))
            },
        );

        StackPanel::new()
            .children((
                TextBlock::new().text(format!("opened: {}", self.opened.unwrap_or("nothing"))),
                ItemsRepeater::new().virtual_source(items),
            ))
            .into()
    }
}

#[guinea::test(iterations = 4)]
fn the_same_mark_in_every_item_is_found_within_the_item(h: &mut Harness) {
    let mut page = Mounted::<CataloguePage>::mount(h.segment(),()).unwrap();
    assert!(page.find(Marks::Open).is_none(), "no item is built before it is in view");

    page.item(3).click(Marks::Open);
    page.settle();
    assert!(page.find_text("opened: gum").is_some(), "{:#?}", page.tree());

    let item = page.item(1);
    assert!(item.find_text("guinea-app").is_some(), "{:#?}", item.tree());
    item.click(Marks::Open);
    page.settle();
    assert!(page.find_text("opened: guinea-app").is_some(), "{:#?}", page.tree());
}

/// What a layout above would provide.
fn shade() -> &'static windows_reactor::Context<&'static str> {
    thread_local! {
        static SHADE: &'static windows_reactor::Context<&'static str> =
            Box::leak(Box::new(windows_reactor::Context::new("light")));
    }
    SHADE.with(|shade| *shade)
}

/// Rows the way a table draws them: the row selects on a release, the
/// chevron inside it toggles on one of its own, and a button removes.
#[derive(Default)]
pub struct RowsPage {
    selected: Option<usize>,
    toggled: Option<usize>,
    removed: Option<usize>,
}

pub enum Rowing {
    Selected(usize),
    Toggled(usize),
    Removed(usize),
}

#[page]
impl Page for RowsPage {
    type Params = ();
    type Message = Rowing;

    fn update(&mut self, message: Rowing, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Rowing::Selected(at) => self.selected = Some(at),
            Rowing::Toggled(at) => self.toggled = Some(at),
            Rowing::Removed(at) => self.removed = Some(at),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let shade = cx.use_context(shade());
        let select = cx.on(Rowing::Selected);
        let toggle = cx.on(Rowing::Toggled);
        let remove = cx.on(Rowing::Removed);

        let rows = VirtualSource::new(
            CATALOGUE.len() as u64,
            CATALOGUE.len(),
            |index| index,
            move |index| {
                let (select, toggle, remove) = (select.clone(), toggle.clone(), remove.clone());

                Border::new()
                    .on_pointer_released(Callback::new(move |_: PointerEventInfo| {
                        let _ = select.call(index);
                    }))
                    .content(StackPanel::new().children((
                        Border::new()
                            .mark(Marks::Chevron)
                            .on_pointer_released(Callback::new(move |_: PointerEventInfo| {
                                let _ = toggle.call(index);
                            }))
                            .content(TextBlock::new().text(">")),
                        TextBlock::new().text(CATALOGUE[index]),
                        Button::new()
                            .mark(Marks::Remove)
                            .is_enabled(index != 0)
                            .on_click(move || {
                                let _ = remove.call(index);
                            })
                            .content(TextBlock::new().text("Remove")),
                    )))
            },
        );

        StackPanel::new()
            .children((
                TextBlock::new().text(format!(
                    "{shade}: selected {:?} toggled {:?} removed {:?}",
                    self.selected, self.toggled, self.removed
                )),
                ItemsRepeater::new().virtual_source(rows),
            ))
            .into()
    }
}

/// A row with a grip that drags it: the grip captures the pointer and follows
/// it by the window's `y`. The capture is lost just before the release, so a
/// lost capture only puts the drag aside: a release after it still drops, and
/// with none it stays cancelled. The row hears the release as it bubbles.
#[derive(Default)]
pub struct GripPage {
    start: Option<f64>,
    lost: bool,
    offset: f64,
    local: f64,
    dropped: Option<f64>,
    heard: Vec<&'static str>,
}

pub enum Gripping {
    Down(PointerEventInfo),
    Moved(PointerEventInfo),
    Up(PointerEventInfo),
    Lost,
    RowUp(PointerEventInfo),
}

#[page]
impl Page for GripPage {
    type Params = ();
    type Message = Gripping;

    fn update(&mut self, message: Gripping, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Gripping::Down(info) => {
                if info.is_left_button_pressed && info.capture_succeeded == Some(true) {
                    self.start = Some(info.window_y);
                }
            }
            Gripping::Moved(info) => {
                if let Some(start) = self.start
                    && info.is_left_button_pressed
                {
                    self.offset = info.window_y - start;
                    self.local = info.y;
                }
            }
            Gripping::Up(_) => {
                self.heard.push("grip");
                if self.start.take().is_some() || std::mem::take(&mut self.lost) {
                    self.dropped = Some(self.offset);
                }
            }
            Gripping::Lost => {
                self.heard.push("lost");
                if self.start.take().is_some() {
                    self.lost = true;
                }
            }
            Gripping::RowUp(_) => self.heard.push("row"),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let lost = cx.on(|()| Gripping::Lost);

        Border::new()
            .on_pointer_released(cx.on(Gripping::RowUp))
            .content(StackPanel::new().children((
                Border::new()
                    .mark(Marks::Grip)
                    .capture_pointer_on_press(true)
                    .on_pointer_pressed(cx.on(Gripping::Down))
                    .on_pointer_moved(cx.on(Gripping::Moved))
                    .on_pointer_released(cx.on(Gripping::Up))
                    .on_pointer_capture_lost(move || {
                        let _ = lost.call(());
                    })
                    .content(TextBlock::new().text("=")),
                TextBlock::new().text(format!(
                    "offset {} local {} dropped {:?} cancelled {} heard {}",
                    self.offset,
                    self.local,
                    self.dropped,
                    self.lost,
                    self.heard.join(",")
                )),
            )))
            .into()
    }
}

#[guinea::test(iterations = 2)]
fn a_drag_follows_the_window_and_drops_on_release(h: &mut Harness) {
    use guinea::winui::harness::Drag;

    let mut page = Mounted::<GripPage>::mount(h.segment(),()).unwrap();

    page.drag(Marks::Grip, Drag::by(0.0, 30.0).from(2.0, 5.0).window_from(10.0, 100.0))
        .settle();
    page.settle();

    let seen = "offset 30 local 35 dropped Some(30.0) cancelled false heard lost,grip,row";
    assert!(page.find_text(seen).is_some(), "{:#?}", page.tree());
}

#[guinea::test(iterations = 2)]
fn a_drag_whose_capture_is_lost_is_cancelled(h: &mut Harness) {
    use guinea::winui::harness::Drag;

    let mut page = Mounted::<GripPage>::mount(h.segment(),()).unwrap();

    page.drag(Marks::Grip, Drag::by(0.0, 30.0).steps(2).lost()).settle();
    page.settle();

    let seen = "offset 30 local 31 dropped None cancelled true heard lost";
    assert!(page.find_text(seen).is_some(), "{:#?}", page.tree());
}

#[guinea::test(iterations = 2)]
fn a_click_bubbles_through_every_listener_and_stops_at_a_button(h: &mut Harness) {
    let mut page =
        Mounted::<RowsPage>::mount_with(h.segment(),(), |page| provide(shade(), "dark", page))
            .unwrap();
    assert!(
        page.find_text("dark: selected None toggled None removed None").is_some(),
        "{:#?}",
        page.tree()
    );

    page.item_with_text("gum").click(Marks::Chevron);
    page.settle();
    assert!(
        page.find_text("dark: selected Some(3) toggled Some(3) removed None").is_some(),
        "{:#?}",
        page.tree()
    );

    page.item(4).click(Marks::Remove);
    page.settle();
    assert!(
        page.find_text("dark: selected Some(3) toggled Some(3) removed Some(4)").is_some(),
        "{:#?}",
        page.tree()
    );
}

#[guinea::test(iterations = 1)]
#[should_panic(expected = "\"Remove\" cannot be clicked: it is inside a disabled Button")]
fn a_disabled_button_does_not_take_the_click(h: &mut Harness) {
    let mut page = Mounted::<RowsPage>::mount(h.segment(),()).unwrap();
    page.item(0).click(Marks::Remove);
}

/// A switch, a check box and a radio button, each showing what the page holds
/// and telling it when a click turns one over.
#[derive(Default)]
pub struct SwitchesPage {
    on: bool,
    checked: bool,
    picked: bool,
}

pub enum Switching {
    Toggled(bool),
    Checked(bool),
    Picked(bool),
}

#[page]
impl Page for SwitchesPage {
    type Params = ();
    type Message = Switching;

    fn update(&mut self, message: Switching, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Switching::Toggled(on) => self.on = on,
            Switching::Checked(checked) => self.checked = checked,
            Switching::Picked(picked) => self.picked = picked,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                ToggleSwitch::new()
                    .mark(Marks::Switch)
                    .is_on(self.on)
                    .on_toggled(cx.on(Switching::Toggled)),
                CheckBox::new()
                    .mark(Marks::Check)
                    .is_checked(self.checked)
                    .on_is_checked_changed(cx.on(|checked| Switching::Checked(checked == Some(true)))),
                RadioButton::new()
                    .mark(Marks::Radio)
                    .is_checked(self.picked)
                    .on_checked(cx.on(|picked| Switching::Picked(picked == Some(true)))),
                TextBlock::new().text(format!(
                    "on {} checked {} picked {}",
                    self.on, self.checked, self.picked
                )),
            ))
            .into()
    }
}

#[guinea::test(iterations = 2)]
fn a_click_turns_a_switch_a_check_box_and_a_radio_button_over(h: &mut Harness) {
    let mut page = Mounted::<SwitchesPage>::mount(h.segment(),()).unwrap();

    page.click(Marks::Switch).settle();
    page.click(Marks::Check).settle();
    page.click(Marks::Radio).settle();
    page.settle();
    assert!(
        page.find_text("on true checked true picked true").is_some(),
        "{:#?}",
        page.tree()
    );

    page.click(Marks::Switch).settle();
    page.click(Marks::Check).settle();
    page.click(Marks::Radio).settle();
    page.settle();
    assert!(
        page.find_text("on false checked false picked true").is_some(),
        "a switch and a check box turn back, a radio button stays on:\n{:#?}",
        page.tree()
    );
}

/// A "…" button whose flyout holds a check box per chart, the way a sidebar
/// picks which charts it shows.
#[derive(Default)]
pub struct ChartsPage {
    disk: bool,
}

pub struct ShowDisk(bool);

#[page]
impl Page for ChartsPage {
    type Params = ();
    type Message = ShowDisk;

    fn update(&mut self, ShowDisk(disk): ShowDisk, _cx: &mut UpdateCx<'_, Self>) {
        self.disk = disk;
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                Button::new().mark(Marks::Charts).content("…").flyout_with(Flyout::rich(
                    CheckBox::new()
                        .mark(Marks::ShowDisk)
                        .is_checked(self.disk)
                        .on_is_checked_changed(cx.on(|checked| ShowDisk(checked == Some(true)))),
                )),
                TextBlock::new().text(format!("disk {}", self.disk)),
            ))
            .into()
    }
}

#[guinea::test(iterations = 2)]
fn a_click_reaches_a_control_inside_a_button_s_flyout(h: &mut Harness) {
    let mut page = Mounted::<ChartsPage>::mount(h.segment(),()).unwrap();
    assert!(page.find(Marks::ShowDisk).is_some(), "{:#?}", page.tree());

    page.click(Marks::ShowDisk).settle();
    page.settle();
    assert!(page.find_text("disk true").is_some(), "{:#?}", page.tree());

    page.click(Marks::ShowDisk).settle();
    page.settle();
    assert!(page.find_text("disk false").is_some(), "{:#?}", page.tree());
}

#[guinea::test(iterations = 2)]
fn a_list_says_how_long_it_is_and_its_items_what_they_hold(h: &mut Harness) {
    let mut page = Mounted::<RowsPage>::mount(h.segment(),()).unwrap();
    assert_eq!(page.item_count(), CATALOGUE.len());

    let items = page.items();
    assert_eq!(items.len(), CATALOGUE.len());
    assert!(items[5].find_text("gust").is_some(), "{:#?}", items[5]);

    let unmarked = items[2].find_text("gui").unwrap().at;
    assert_eq!(
        page.at(unmarked).property(PropertyId::Text),
        Some(&PropertyValue::String("gui".into()))
    );

    let first = page.item(0).find(Marks::Remove).unwrap();
    let second = page.item(1).find(Marks::Remove).unwrap();
    assert_eq!(
        page.property(first, PropertyId::IsEnabled),
        Some(&PropertyValue::Bool(false))
    );
    assert_ne!(
        page.property(second, PropertyId::IsEnabled),
        Some(&PropertyValue::Bool(false))
    );
}

/// Samples on a timer; installed into the segment above the page.
mod polling {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Samples {
        pub taken: u32,
    }

    #[derive(Clone, Debug)]
    pub struct Taken;

    #[reducer]
    fn samples(this: &mut Samples, _taken: Taken) {
        this.taken += 1;
    }

    #[derive(Clone, Debug)]
    pub struct Sample;

    #[derive(Debug)]
    pub struct Sampler {
        pub push: Push<Samples>,
    }

    actor! {
        Sampler {
            handlers { Sample }
        }
    }

    #[handler]
    fn sample(this: &mut Sampler, _: Sample) {
        this.push.send(Taken);
    }

    feature! {
        pub Polling {
            exports { Samples }
        }
    }

    #[installs]
    fn polling(cx: &FeatureInitContext) -> anyhow::Result<Polling> {
        let (samples, sampler) = cx.state::<Samples>().driven_by(|push| Sampler { push });
        cx.every(std::time::Duration::from_millis(100), || Sample, sampler);
        Ok(Polling(samples))
    }
}

thread_local! {
    static MEASURED: std::cell::RefCell<Vec<(f64, f64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Draws on a composition host, as a chart does, and learns its size from it.
pub struct PlotPage {
    host: ElementRef<Grid>,
    _measured: ElementObservation,
}

impl Default for PlotPage {
    fn default() -> Self {
        let host = ElementRef::<Grid>::new();
        let measured = host.observe_composition_host(|event| {
            if let CompositionHostEvent::Metrics { width, height, .. } = event {
                MEASURED.with(|seen| seen.borrow_mut().push((width, height)));
            }
        });

        Self {
            host,
            _measured: measured,
        }
    }
}

#[page]
impl Page for PlotPage {
    type Params = ();
    type Message = ();

    fn update(&mut self, _message: (), _cx: &mut UpdateCx<'_, Self>) {}

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        Border::new()
            .mark(Marks::Plot)
            .content(Grid::new().element_ref(&self.host))
            .into()
    }
}

#[guinea::test(iterations = 2)]
fn a_composition_host_hears_the_size_its_element_is_given(h: &mut Harness) {
    MEASURED.with(|seen| seen.borrow_mut().clear());
    let mut page = Mounted::<PlotPage>::mount(h.segment(), ()).unwrap();
    assert!(MEASURED.with(|seen| seen.borrow().is_empty()), "no size before the test gives one");

    page.size(Marks::Plot, 300.0, 120.0);

    assert_eq!(MEASURED.with(|seen| seen.borrow().clone()), [(300.0, 120.0)]);
}

/// Shows how many samples the segment above it has taken.
#[derive(Default)]
pub struct SamplesPage;

#[page]
impl Page for SamplesPage {
    type Params = ();
    type Message = ();

    fn update(&mut self, _message: (), _cx: &mut UpdateCx<'_, Self>) {}

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (samples, _) = cx.read::<polling::Samples>();
        TextBlock::new().text(format!("samples: {}", samples.taken)).into()
    }
}

#[guinea::test(iterations = 8)]
fn a_page_redraws_when_the_segment_above_it_changes(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();

    let below = h.child();
    let mut page = Mounted::<SamplesPage>::mount(below, ()).unwrap();
    assert!(page.find_text("samples: 0").is_some(), "{:#?}", page.tree());

    h.advance(std::time::Duration::from_millis(300));
    page.settle();

    assert!(page.find_text("samples: 3").is_some(), "{:#?}", page.tree());
}

fn samples_below(h: &Harness) -> Mounted<'_, SamplesPage> {
    Mounted::mount(h.child(), ()).unwrap()
}

#[guinea::test(iterations = 4)]
fn a_page_keeps_the_segment_it_was_mounted_into(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();
    let mut page = samples_below(h);

    h.advance(std::time::Duration::from_millis(300));
    let settled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| page.settle()));

    assert!(settled.is_ok(), "the page's segment was torn down under it");
    assert!(page.find_text("samples: 3").is_some(), "{:#?}", page.tree());
}

mod routed {
    use super::*;
    use guinea::winui::{Layout, LayoutCx, Routable, layout};
    use std::rc::Rc;

    use windows_reactor::{NavigationView, NavigationViewItem, keyed};

    #[derive(Default)]
    pub struct One;

    #[page]
    impl Page for One {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("one").into()
        }
    }

    #[derive(Default)]
    pub struct Two;

    #[page]
    impl Page for Two {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("two").into()
        }
    }

    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum TabRoute {
        One,
        Two,
    }

    /// Shows where the application is, and asks to go elsewhere.
    #[derive(Default)]
    pub struct Tabs;

    #[layout]
    impl Layout for Tabs {
        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<TabRoute>();
            let here = match nav.current() {
                Some(TabRoute::One) => "at one",
                Some(TabRoute::Two) => "at two",
                None => "nowhere",
            };

            StackPanel::new()
                .children((
                    TextBlock::new().text(here),
                    Button::new()
                        .on_click(nav.to_handler(TabRoute::Two))
                        .content(TextBlock::new().text("Two")),
                    cx.outlet(&nav, |tab, cx| match tab {
                        TabRoute::One => cx.mount::<One>(&()),
                        TabRoute::Two => cx.mount::<Two>(&()),
                    }),
                ))
                .into()
        }
    }

    #[guinea::test(iterations = 4)]
    fn a_layout_mounted_at_a_route_draws_it_and_goes_where_it_is_asked(h: &mut Harness) {
        let mut tabs = Mounted::<Tabs>::mount_at(h.segment(), (), TabRoute::One).unwrap();
        assert!(tabs.find_text("at one").is_some(), "{:#?}", tabs.tree());
        assert!(tabs.find_text("one").is_some(), "{:#?}", tabs.tree());

        tabs.click_text("Two").settle();
        tabs.settle();

        assert_eq!(tabs.navigator::<TabRoute>().current(), Some(TabRoute::Two));
        assert!(tabs.find_text("at two").is_some(), "{:#?}", tabs.tree());
        assert!(tabs.find_text("two").is_some(), "{:#?}", tabs.tree());
    }

    #[derive(Default)]
    pub struct Home;

    #[page]
    impl Page for Home {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("home").into()
        }
    }

    #[derive(Default)]
    pub struct Settings;

    #[page]
    impl Page for Settings {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("settings").into()
        }
    }

    /// A navigation pane: a menu item, a footer item, and the outlet as its
    /// content.
    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum PaneRoute {
        Home,
        Settings,
    }

    #[derive(Default)]
    pub struct Pane;

    #[layout]
    impl Layout for Pane {
        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<PaneRoute>();
            let page = cx.outlet(&nav, |place, cx| match place {
                PaneRoute::Home => cx.mount::<Home>(&()),
                PaneRoute::Settings => cx.mount::<Settings>(&()),
            });

            NavigationView::new()
                .keyed_menu_items([keyed(
                    "home",
                    NavigationViewItem::new().tag("home").content("Home"),
                )])
                .keyed_footer_menu_items([keyed(
                    "settings",
                    NavigationViewItem::new().tag("settings").content("Settings"),
                )])
                .on_selected_tag_changed(move |tag: Option<Rc<str>>| match tag.as_deref() {
                    Some("home") => {
                        nav.to(PaneRoute::Home);
                    }
                    Some("settings") => {
                        nav.to(PaneRoute::Settings);
                    }
                    _ => {}
                })
                .content(page)
                .into()
        }
    }

    #[guinea::test(iterations = 4)]
    fn a_navigation_view_is_walked_and_its_items_select_themselves(h: &mut Harness) {
        let mut pane = Mounted::<Pane>::mount_at(h.segment(), (), PaneRoute::Home).unwrap();

        assert!(pane.find_text("home").is_some(), "{:#?}", pane.tree());
        assert!(pane.find_text("Home").is_some(), "{:#?}", pane.tree());

        pane.click_text("Settings").settle();
        pane.settle();

        assert_eq!(pane.navigator::<PaneRoute>().current(), Some(PaneRoute::Settings));
        assert!(pane.find_text("settings").is_some(), "{:#?}", pane.tree());
    }
}

mod navigating {
    use std::cell::Cell;
    use std::time::Duration;

    use super::*;
    use guinea::winui::{Ask, Layout, LayoutCx, Routable, Verdict, layout};

    thread_local! {
        static AREAS_GONE: Cell<usize> = const { Cell::new(0) };
        static KEPT_WAKES: Cell<usize> = const { Cell::new(0) };
    }

    pub mod frame_clock {
        use guinea::prelude::*;

        #[derive(Default, Clone, PartialEq, Debug)]
        pub struct FrameTicks(pub u32);

        #[derive(Clone, Debug)]
        pub struct Tick;

        #[reducer]
        fn frame_ticks(this: &mut FrameTicks, _tick: Tick) {
            this.0 += 1;
        }

        #[derive(Debug)]
        pub struct FrameTicker {
            pub push: Push<FrameTicks>,
        }

        actor! {
            FrameTicker {
                handlers { Tick }
            }
        }

        #[handler]
        fn frame_tick(this: &mut FrameTicker, tick: Tick) {
            this.push.send(tick);
        }

        feature! {
            pub FrameClock {
                exports { FrameTicks }
            }
        }

        #[installs]
        fn frame_clock(cx: &FeatureInitContext) -> anyhow::Result<FrameClock> {
            let (ticks, ticker) = cx.state::<FrameTicks>().driven_by(|push| FrameTicker { push });
            cx.every(std::time::Duration::from_secs(1), || Tick, ticker);
            Ok(FrameClock(ticks))
        }
    }

    pub mod list_clock {
        use guinea::prelude::*;

        #[derive(Default, Clone, PartialEq, Debug)]
        pub struct ListTicks(pub u32);

        #[derive(Clone, Debug)]
        pub struct Tick;

        #[reducer]
        fn list_ticks(this: &mut ListTicks, _tick: Tick) {
            this.0 += 1;
        }

        #[derive(Debug)]
        pub struct ListTicker {
            pub push: Push<ListTicks>,
        }

        actor! {
            ListTicker {
                handlers { Tick }
            }
        }

        #[handler]
        fn list_tick(this: &mut ListTicker, tick: Tick) {
            this.push.send(tick);
        }

        feature! {
            pub ListClock {
                exports { ListTicks }
            }
        }

        #[installs]
        fn list_clock(cx: &FeatureInitContext) -> anyhow::Result<ListClock> {
            let (ticks, ticker) = cx.state::<ListTicks>().driven_by(|push| ListTicker { push });
            cx.every(std::time::Duration::from_secs(1), || Tick, ticker);
            Ok(ListClock(ticks))
        }
    }

    #[derive(Clone)]
    pub struct Greeting(pub &'static str);

    pub mod greeted {
        use guinea::prelude::*;

        use super::Greeting;

        #[derive(Default, Clone, PartialEq, Debug)]
        pub struct Heard(pub &'static str);

        #[reducer]
        fn heard(this: &mut Heard, greeting: &'static str) {
            this.0 = greeting;
        }

        feature! {
            pub Greeted {
                exports { Heard }
            }
        }

        #[installs]
        fn greeted(cx: &FeatureInitContext, greeting: Option<Greeting>) -> anyhow::Result<Greeted> {
            let greeting = greeting.map_or("nobody", |greeting| greeting.0);
            Ok(Greeted(cx.state::<Heard>().seed(Heard(greeting)).plain()))
        }
    }

    /// The shell: where the application is, a way to the list, and a clock
    /// that lives as long as the shell does.
    #[derive(Default)]
    pub struct Frame;

    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum AppRoute {
        List,
        KeptList,
        Other,
        Draft,
    }

    #[layout]
    impl Layout for Frame {
        type Installs = frame_clock::FrameClock;

        fn install(
            ctx: &FeatureInitContext,
            _params: &(),
        ) -> anyhow::Result<(Self, frame_clock::FrameClock)> {
            Ok((Self::default(), ctx.install(&())?))
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<AppRoute>();
            let here = match nav.current() {
                Some(AppRoute::List) => "at list",
                Some(AppRoute::Other) => "at other",
                Some(AppRoute::Draft) => "at draft",
                Some(AppRoute::KeptList) => "at kept list",
                None => "nowhere",
            };
            let (ticks, _) = cx.read::<frame_clock::FrameTicks>();

            StackPanel::new()
                .children((
                    TextBlock::new().text(here),
                    TextBlock::new().text(format!("frame ticks: {}", ticks.0)),
                    Button::new()
                        .on_click(nav.to_handler(AppRoute::List))
                        .content(TextBlock::new().text("List")),
                    cx.outlet(&nav, |place, cx| match place {
                        AppRoute::List => cx.mount::<Area>(&()),
                        AppRoute::KeptList => cx.mount::<KeptArea>(&()).keep(),
                        AppRoute::Other => cx.mount::<Other>(&()),
                        AppRoute::Draft => cx.mount::<Draft>(&()),
                    }),
                ))
                .into()
        }
    }

    /// A layout below the shell that holds the list's clock - the place a
    /// feature is lost from when the list is left.
    #[derive(Default)]
    pub struct Area;

    #[layout]
    impl Layout for Area {
        type Installs = list_clock::ListClock;

        fn install(
            ctx: &FeatureInitContext,
            _params: &(),
        ) -> anyhow::Result<(Self, list_clock::ListClock)> {
            ctx.scope
                .on_teardown(|| AREAS_GONE.with(|gone| gone.set(gone.get() + 1)));
            Ok((Self::default(), ctx.install(&())?))
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            cx.mount::<List>(&()).into()
        }
    }

    #[derive(Default)]
    pub struct List;

    #[page]
    impl Page for List {
        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (ticks, _) = cx.read::<list_clock::ListTicks>();
            TextBlock::new().text(format!("list ticks: {}", ticks.0)).into()
        }
    }

    /// Reads what the harness provided, in a page installed only after a
    /// navigation.
    #[derive(Default)]
    pub struct Other;

    #[page]
    impl Page for Other {
        type Installs = greeted::Greeted;

        fn install(
            ctx: &FeatureInitContext,
            _params: &(),
        ) -> anyhow::Result<(Self, greeted::Greeted)> {
            Ok((Self, ctx.install(&())?))
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (heard, _) = cx.read::<greeted::Heard>();
            TextBlock::new().text(format!("other says {}", heard.0)).into()
        }
    }

    #[derive(Default)]
    pub struct Draft;

    #[page]
    impl Page for Draft {
        fn leaving(&self) -> Verdict {
            Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep"))
        }

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("draft").into()
        }
    }

    /// The same clock as [`Area`], in a layout the frame keeps.
    #[derive(Default)]
    pub struct KeptArea;

    #[layout]
    impl Layout for KeptArea {
        type Installs = list_clock::ListClock;

        fn install(
            ctx: &FeatureInitContext,
            _params: &(),
        ) -> anyhow::Result<(Self, list_clock::ListClock)> {
            ctx.on_wake(|| KEPT_WAKES.with(|wakes| wakes.set(wakes.get() + 1)));
            Ok((Self::default(), ctx.install(&())?))
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            cx.mount::<KeptList>(&()).into()
        }
    }

    #[derive(Default)]
    pub struct KeptList;

    #[page]
    impl Page for KeptList {
        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (ticks, _) = cx.read::<list_clock::ListTicks>();
            TextBlock::new().text(format!("kept ticks: {}", ticks.0)).into()
        }
    }

    #[guinea::test(iterations = 4)]
    fn a_navigation_tears_down_what_it_left_and_installs_what_it_reached(h: &mut Harness) {
        h.provide(Greeting("hello"));
        AREAS_GONE.with(|gone| gone.set(0));

        let mut app = Mounted::routed::<Frame>(h, AppRoute::List).unwrap();
        assert_eq!(app.route(), AppRoute::List);
        assert!(app.is_mounted::<Frame>() && app.is_mounted::<Area>() && app.is_mounted::<List>());
        assert!(app.find_text("at list").is_some(), "{:#?}", app.tree());

        app.navigate(AppRoute::Other);
        assert_eq!(app.route(), AppRoute::Other);
        assert!(!app.is_mounted::<Area>() && !app.is_mounted::<List>(), "{:?}", app.segments());
        assert_eq!(AREAS_GONE.with(Cell::get), 1, "the area's scope went with it");
        assert!(app.find_text("at other").is_some(), "{:#?}", app.tree());
        assert!(
            app.find_text("other says hello").is_some(),
            "what the harness provides reaches a page installed later:\n{:#?}",
            app.tree()
        );

        app.click_text("List").settle();
        app.settle();
        assert_eq!(app.route(), AppRoute::List, "a click through the navigator moves too");
        assert!(app.is_mounted::<Area>(), "{:?}", app.segments());
        assert!(app.find_text("list ticks: 0").is_some(), "{:#?}", app.tree());
    }

    #[guinea::test(iterations = 4)]
    fn state_below_the_page_left_starts_again_and_state_above_it_stays(h: &mut Harness) {
        let mut app = Mounted::routed::<Frame>(h, AppRoute::List).unwrap();

        h.advance(Duration::from_secs(1));
        app.settle();
        assert!(app.find_text("frame ticks: 1").is_some(), "{:#?}", app.tree());
        assert!(app.find_text("list ticks: 1").is_some(), "{:#?}", app.tree());

        app.navigate(AppRoute::Other);
        app.navigate(AppRoute::List);

        assert!(app.find_text("frame ticks: 1").is_some(), "{:#?}", app.tree());
        assert!(
            app.find_text("list ticks: 0").is_some(),
            "the area was left with the list, and its clock with it:\n{:#?}",
            app.tree()
        );
    }

    #[guinea::test(iterations = 4)]
    fn a_guard_s_question_is_answered_through_the_root_s_dialog(h: &mut Harness) {
        let mut app = Mounted::routed::<Frame>(h, AppRoute::Draft).unwrap();
        assert_eq!(app.question(), None);

        app.navigate(AppRoute::Other);
        assert_eq!(app.question().as_deref(), Some("Discard the draft?"));
        assert_eq!(app.route(), AppRoute::Draft);

        app.answer(false);
        assert_eq!(app.question(), None);
        assert_eq!(app.route(), AppRoute::Draft, "kept");

        app.navigate(AppRoute::Other);
        app.answer(true);
        assert_eq!(app.route(), AppRoute::Other, "discarded");
        assert!(app.find_text("at other").is_some(), "{:#?}", app.tree());
    }

    #[guinea::test(iterations = 4)]
    fn a_kept_layout_comes_back_with_its_state_and_its_clock_slept_meanwhile(h: &mut Harness) {
        KEPT_WAKES.with(|wakes| wakes.set(0));
        let mut app = Mounted::routed::<Frame>(h, AppRoute::KeptList).unwrap();

        h.advance(Duration::from_secs(1));
        app.settle();
        assert!(app.find_text("kept ticks: 1").is_some(), "{:#?}", app.tree());

        app.navigate(AppRoute::Other);
        assert!(!app.is_mounted::<KeptArea>(), "{:?}", app.segments());
        h.advance(Duration::from_secs(3));
        app.settle();
        assert_eq!(KEPT_WAKES.with(Cell::get), 0);

        app.navigate(AppRoute::KeptList);
        assert!(
            app.find_text("kept ticks: 1").is_some(),
            "the area kept its state, and its clock did not tick while it slept:\n{:#?}",
            app.tree()
        );
        assert_eq!(KEPT_WAKES.with(Cell::get), 1, "woken once, on the way back");

        h.advance(Duration::from_secs(1));
        app.settle();
        assert!(
            app.find_text("kept ticks: 2").is_some(),
            "awake, the clock ticks again:\n{:#?}",
            app.tree()
        );
    }
}

mod application_exports {
    use guinea::feature::Reads;

    use super::*;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Language(pub &'static str);

    #[reducer]
    fn language(this: &mut Language, to: &'static str) {
        this.0 = to;
    }

    pub struct Localisation;

    impl AppFeature for Localisation {
        type Exports = (Language,);

        fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
            let language = app.state::<Language>().seed(Language("en")).plain();
            app.answers(move |Speak(to)| language.push(to));
            Ok(())
        }
    }

    pub struct Speak(&'static str);

    #[derive(Default)]
    pub struct Greeting;

    #[page]
    impl Page for Greeting {
        type Params = ();

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (language, _) = cx.read::<Language>();
            TextBlock::new().text(format!("speaks {}", language.0)).into()
        }
    }

    #[guinea::test(iterations = 2)]
    fn a_page_reads_what_the_application_exports(h: &mut Harness) {
        h.feature(Localisation).unwrap();

        let page = Mounted::<Greeting>::mount(h.segment(),()).unwrap();

        assert!(page.find_text("speaks en").is_some(), "{:#?}", page.tree());
    }

    /// A plugin's shortcut, written once for every backend.
    trait Speaks: Reads {
        fn language(&mut self) -> &'static str {
            self.read::<Language>().0.0
        }
    }

    impl<C: Reads> Speaks for C {}

    #[derive(Default)]
    pub struct Polyglot;

    #[page]
    impl Page for Polyglot {
        type Params = ();

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text(format!("speaks {}", cx.language())).into()
        }
    }

    #[guinea::test(iterations = 2)]
    fn a_shortcut_written_over_reads_reads_on_a_page(h: &mut Harness) {
        h.feature(Localisation).unwrap();

        let page = Mounted::<Polyglot>::mount(h.segment(),()).unwrap();

        assert!(page.find_text("speaks en").is_some(), "{:#?}", page.tree());
    }

    #[derive(Default)]
    pub struct Switcher;

    #[page]
    impl Page for Switcher {
        type Params = ();

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let speak = Reads::dispatch::<Language>(cx);
            Button::new()
                .mark(Marks::Speak)
                .on_click(move || speak.emit(Speak("ru")))
                .content(TextBlock::new().text("speak"))
                .into()
        }
    }

    #[guinea::test(iterations = 2)]
    fn a_page_asks_what_the_application_exports_without_reading_it(h: &mut Harness) {
        h.feature(Localisation).unwrap();

        let mut switcher = Mounted::<Switcher>::mount(h.segment(),()).unwrap();
        switcher.click(Marks::Speak).settle();

        let page = Mounted::<Greeting>::mount(h.segment(),()).unwrap();
        assert!(page.find_text("speaks ru").is_some(), "{:#?}", page.tree());
    }
}

/// A title bar with something in its left header, which the harness reads
/// like any other child.
#[derive(Default)]
pub struct Titled;

#[page]
impl Page for Titled {
    type Params = ();

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        windows_reactor::TitleBar::new()
            .title("uniproc")
            .left_header(TextBlock::new().text("left of the title"))
            .into()
    }
}

#[guinea::test(iterations = 2)]
fn what_a_title_bar_holds_on_its_left_is_found(h: &mut Harness) {
    let page = Mounted::<Titled>::mount(h.segment(), ()).unwrap();

    assert!(page.find_text("left of the title").is_some(), "{:#?}", page.tree());
}

/// Two of the same mounted at once: what is done to one is done to that one.
mod two_of_a_kind {
    use super::*;

    #[derive(Default)]
    pub struct Tally {
        n: u32,
    }

    pub enum Add {
        One,
    }

    #[page]
    impl Page for Tally {
        type Message = Add;

        fn update(&mut self, message: Add, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Add::One => self.n += 1,
            }
        }

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text(format!("tally {}", self.n)).into()
        }
    }

    #[guinea::test(iterations = 1)]
    fn send_reaches_the_page_it_is_called_on(h: &mut Harness) {
        let mut first = Mounted::<Tally>::mount(h.segment(), ()).unwrap();
        let mut second = Mounted::<Tally>::mount(h.segment(), ()).unwrap();

        first.send(Add::One);
        first.settle();
        second.settle();

        let shown = |page: &Mounted<'_, Tally>| {
            ["tally 0", "tally 1"]
                .into_iter()
                .find(|text| page.find_text(text).is_some())
        };
        assert_eq!((shown(&first), shown(&second)), (Some("tally 1"), Some("tally 0")));
    }

    #[derive(Clone, PartialEq, Debug, guinea::winui::Routable)]
    pub enum Where {
        Here,
        There,
    }

    #[derive(Default)]
    pub struct Window;

    #[guinea::winui::layout]
    impl guinea::winui::Layout for Window {
        fn view(&self, cx: &mut guinea::winui::LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<Where>();
            cx.outlet(&nav, |place, cx| match place {
                Where::Here => cx.mount::<Here>(&()),
                Where::There => cx.mount::<There>(&()),
            })
        }
    }

    #[derive(Default)]
    pub struct Here;

    #[page]
    impl Page for Here {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("here").into()
        }
    }

    #[derive(Default)]
    pub struct There;

    #[page]
    impl Page for There {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("there").into()
        }
    }

    #[guinea::test(iterations = 1)]
    fn navigate_moves_the_window_it_is_called_on(h: &mut Harness) {
        let mut first = Mounted::routed::<Window>(h, Where::Here).unwrap();
        let second = Mounted::routed::<Window>(h, Where::Here).unwrap();

        first.navigate(Where::There);

        assert_eq!((first.route(), second.route()), (Where::There, Where::Here));
    }
}
