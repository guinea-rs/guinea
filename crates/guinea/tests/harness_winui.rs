#![cfg(feature = "winui")]

//! A WinUI page under the harness: mounted with no window, typed into and
//! clicked, and read back as the tree it drew - with the features behind it
//! running in the seed's order.

use guinea::app::Harness;
use guinea::feature::Segment;
use guinea::prelude::*;
use guinea::winui::harness::{Mounted, PropertyId, PropertyValue};
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};
use windows_reactor::{
    Border, Button, Callback, ChildrenControl, ContentControl, ItemsRepeater, PointerEventInfo,
    StackPanel, TextBlock, View, VirtualSource,
};

const CATALOGUE: [&str; 6] = ["guinea", "guinea-app", "gui", "gum", "gulp", "gust"];

#[derive(guinea::Mark)]
enum Marks {
    Search,
    Open,
    Chevron,
    Remove,
    Grip,
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

impl Reducer for Results {
    type Update = Found;

    fn reduce(&mut self, update: Found) {
        self.query = update.query;
        self.found = update.found;
    }
}

#[derive(Clone, Debug)]
pub struct Query(pub String);

#[derive(Debug)]
pub struct Searcher {
    pub push: Push<Results>,
}

actor! {
    Searcher {
        handlers { Query => { bg Found }, Found }
    }
}

#[handler]
fn query(_this: &mut Searcher, Query(text): Query, cx: Cx) {
    cx.spawn_bg::<Found, _>(async move {
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

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Search> {
        ctx.install(&())
    }

    fn update(&mut self, message: Typing, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Typing::Typed(text) => self.typed = text,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        let (results, dispatch) = cx.use_reducer::<Results, _>();
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

/// What `routes!` would write: the page stands alone.
impl Segment for SearchPage {
    type Installs = Search;
    type Above = ();
}

#[guinea::test(iterations = 30)]
fn typing_and_clicking_search_draws_what_it_found(h: &mut Harness) {
    let mut page = Mounted::<SearchPage>::mount(&h.segment(), ()).unwrap();
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
    let mut page = Mounted::<SearchPage>::mount(&h.segment(), ()).unwrap();
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
    type Installs = ();
    type Message = Browsing;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn update(&mut self, message: Browsing, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Browsing::Opened(at) => self.opened = Some(CATALOGUE[at]),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
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

impl Segment for CataloguePage {
    type Installs = ();
    type Above = ();
}

#[guinea::test(iterations = 4)]
fn the_same_mark_in_every_item_is_found_within_the_item(h: &mut Harness) {
    let mut page = Mounted::<CataloguePage>::mount(&h.segment(), ()).unwrap();
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
    type Installs = ();
    type Message = Rowing;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn update(&mut self, message: Rowing, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Rowing::Selected(at) => self.selected = Some(at),
            Rowing::Toggled(at) => self.toggled = Some(at),
            Rowing::Removed(at) => self.removed = Some(at),
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
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

impl Segment for RowsPage {
    type Installs = ();
    type Above = ();
}

/// A row with a grip that drags it: the grip captures the pointer, follows it
/// by the window's `y`, drops on a release and gives up when the capture is
/// lost first. The row itself hears the release as it bubbles.
#[derive(Default)]
pub struct GripPage {
    start: Option<f64>,
    offset: f64,
    local: f64,
    dropped: Option<f64>,
    cancelled: bool,
    row_released: u32,
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
    type Installs = ();
    type Message = Gripping;

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn update(&mut self, message: Gripping, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Gripping::Down(info) => {
                if info.is_left_button_pressed && info.capture_succeeded {
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
                if self.start.take().is_some() {
                    self.dropped = Some(self.offset);
                }
            }
            Gripping::Lost => {
                if self.start.take().is_some() {
                    self.cancelled = true;
                }
            }
            Gripping::RowUp(_) => self.row_released += 1,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
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
                    "offset {} local {} dropped {:?} cancelled {} row {}",
                    self.offset, self.local, self.dropped, self.cancelled, self.row_released
                )),
            )))
            .into()
    }
}

impl Segment for GripPage {
    type Installs = ();
    type Above = ();
}

#[guinea::test(iterations = 2)]
fn a_drag_follows_the_window_and_drops_on_release(h: &mut Harness) {
    use guinea::winui::harness::Drag;

    let mut page = Mounted::<GripPage>::mount(&h.segment(), ()).unwrap();

    page.drag(Marks::Grip, Drag::by(0.0, 30.0).from(2.0, 5.0).window_from(10.0, 100.0))
        .settle();
    page.settle();

    let seen = "offset 30 local 35 dropped Some(30.0) cancelled false row 1";
    assert!(page.find_text(seen).is_some(), "{:#?}", page.tree());
}

#[guinea::test(iterations = 2)]
fn a_drag_whose_capture_is_lost_is_cancelled(h: &mut Harness) {
    use guinea::winui::harness::Drag;

    let mut page = Mounted::<GripPage>::mount(&h.segment(), ()).unwrap();

    page.drag(Marks::Grip, Drag::by(0.0, 30.0).steps(2).lost()).settle();
    page.settle();

    let seen = "offset 30 local 31 dropped None cancelled true row 0";
    assert!(page.find_text(seen).is_some(), "{:#?}", page.tree());
}

#[guinea::test(iterations = 2)]
fn a_click_bubbles_through_every_listener_and_stops_at_a_button(h: &mut Harness) {
    let mut page =
        Mounted::<RowsPage>::mount_with(&h.segment(), (), |page| View::provide(shade(), "dark", page))
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
    let mut page = Mounted::<RowsPage>::mount(&h.segment(), ()).unwrap();
    page.item(0).click(Marks::Remove);
}

#[guinea::test(iterations = 2)]
fn a_list_says_how_long_it_is_and_its_items_what_they_hold(h: &mut Harness) {
    let mut page = Mounted::<RowsPage>::mount(&h.segment(), ()).unwrap();
    assert_eq!(page.item_count(), CATALOGUE.len());

    let items = page.items();
    assert_eq!(items.len(), CATALOGUE.len());
    assert!(items[5].find_text("gust").is_some(), "{:#?}", items[5]);

    let unmarked = items[2].find_text("gui").unwrap().at;
    assert_eq!(
        page.at(unmarked).property(PropertyId::TextBlockText),
        Some(&PropertyValue::Str("gui".into()))
    );

    let first = page.item(0).find(Marks::Remove).unwrap();
    let second = page.item(1).find(Marks::Remove).unwrap();
    assert_eq!(
        page.property(first, PropertyId::ButtonIsEnabled),
        Some(&PropertyValue::Bool(false))
    );
    assert_ne!(
        page.property(second, PropertyId::ButtonIsEnabled),
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

    impl Reducer for Samples {
        type Update = Taken;

        fn reduce(&mut self, _taken: Taken) {
            self.taken += 1;
        }
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
        cx.every(std::time::Duration::from_millis(100), &sampler, || Sample);
        Ok(Polling(samples))
    }
}

/// The segment above the page, as `routes!` would name it.
pub struct Shell;

impl Segment for Shell {
    type Installs = polling::Polling;
    type Above = ();
}

/// Shows how many samples the segment above it has taken.
#[derive(Default)]
pub struct SamplesPage;

#[page]
impl Page for SamplesPage {
    type Params = ();
    type Installs = ();
    type Message = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn update(&mut self, _message: (), _cx: &mut UpdateCx<'_, Self>) {}

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        let (samples, _) = cx.use_reducer::<polling::Samples, _>();
        TextBlock::new().text(format!("samples: {}", samples.taken)).into()
    }
}

impl Segment for SamplesPage {
    type Installs = ();
    type Above = (Shell, ());
}

#[guinea::test(iterations = 8)]
fn a_page_redraws_when_the_segment_above_it_changes(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();

    let below = h.child();
    let mut page = Mounted::<SamplesPage>::mount(&below, ()).unwrap();
    assert!(page.find_text("samples: 0").is_some(), "{:#?}", page.tree());

    h.advance(std::time::Duration::from_millis(300));
    page.settle();

    assert!(page.find_text("samples: 3").is_some(), "{:#?}", page.tree());
}

mod routed {
    use super::*;
    use guinea::winui::harness::Outlet;
    use guinea::winui::{Layout, LayoutCx, UseRoute, layout};
    use windows_reactor::{NavigationView, NavigationViewItem};

    #[derive(Default)]
    pub struct One;

    #[page]
    impl Page for One {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("one").into()
        }
    }

    #[derive(Default)]
    pub struct Two;

    #[page]
    impl Page for Two {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("two").into()
        }
    }

    /// Shows where the application is, and asks to go elsewhere.
    #[derive(Default)]
    pub struct Tabs;

    #[layout]
    impl Layout for Tabs {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            let here = match cx.use_route::<TabRoute>() {
                TabRoute::One {} => "at one",
                TabRoute::Two {} => "at two",
            };
            let nav = cx.navigate::<TabRoute>();

            StackPanel::new()
                .children((
                    TextBlock::new().text(here),
                    Button::new()
                        .on_click(nav.to_handler(TabRoute::Two {}))
                        .content(TextBlock::new().text("Two")),
                    cx.outlet(),
                ))
                .into()
        }
    }

    routes! {
        TabRoute {
            layout(Tabs) {
                page(One) { }
                page(Two) { }
            }
        }
    }

    #[guinea::test(iterations = 4)]
    fn a_layout_mounted_at_a_route_reads_it_and_records_where_it_asks_to_go(h: &mut Harness) {
        let mut tabs = Mounted::<Tabs>::mount_at(&h.segment(), (), TabRoute::One {}).unwrap();
        assert!(tabs.find_text("at one").is_some(), "{:#?}", tabs.tree());
        assert!(tabs.find(Outlet).is_some(), "{:#?}", tabs.tree());

        tabs.click_text("Two").settle();
        tabs.settle();

        assert_eq!(tabs.navigated::<TabRoute>(), [TabRoute::Two {}]);
        assert!(tabs.find_text("at one").is_some(), "{:#?}", tabs.tree());
    }

    #[derive(Default)]
    pub struct Home;

    #[page]
    impl Page for Home {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("home").into()
        }
    }

    #[derive(Default)]
    pub struct Settings;

    #[page]
    impl Page for Settings {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("settings").into()
        }
    }

    /// A navigation pane: a menu item, a footer item, and the outlet as its
    /// content.
    #[derive(Default)]
    pub struct Pane;

    #[layout]
    impl Layout for Pane {
        type Params = ();
        type Installs = ();

        fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
            Ok(())
        }

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            let nav = cx.navigate::<PaneRoute>();

            NavigationView::new()
                .menu_items([("home", NavigationViewItem::new().tag("home").content("Home"))])
                .footer_menu_items([(
                    "settings",
                    NavigationViewItem::new().tag("settings").content("Settings"),
                )])
                .on_selected_tag_changed(move |tag: Option<String>| match tag.as_deref() {
                    Some("home") => nav.to(PaneRoute::Home {}),
                    Some("settings") => nav.to(PaneRoute::Settings {}),
                    _ => {}
                })
                .content(cx.outlet())
                .into()
        }
    }

    routes! {
        PaneRoute {
            layout(Pane) {
                page(Home) { }
                page(Settings) { }
            }
        }
    }

    #[guinea::test(iterations = 4)]
    fn a_navigation_view_is_walked_and_its_items_select_themselves(h: &mut Harness) {
        let mut pane = Mounted::<Pane>::mount_at(&h.segment(), (), PaneRoute::Home {}).unwrap();

        assert!(pane.find(Outlet).is_some(), "{:#?}", pane.tree());
        assert!(pane.find_text("Home").is_some(), "{:#?}", pane.tree());

        pane.click_text("Settings").settle();
        pane.settle();

        assert_eq!(pane.navigated::<PaneRoute>(), [PaneRoute::Settings {}]);
    }
}
