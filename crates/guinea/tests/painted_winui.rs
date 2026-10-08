#![cfg(all(windows, feature = "winui"))]

//! An element that paints its own rows, read and clicked under the harness
//! through the tree it publishes - there is no XAML under it to read.

use std::cell::Cell;
use std::rc::Rc;

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::{Drag, Mounted};
use guinea::winui::semantics::{Bounds, Items, Published, Role, Semantic, publish};
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};
use windows_reactor::{Border, ElementRef, Grid, PointerEventInfo, StackPanel, TextBlock, View};

const SERVICES: [&str; 6] = ["Audio", "Spooler", "Themes", "Time", "Update", "Wlan"];

const ROW: f64 = 20.0;
const SHOWN: usize = 3;
const NAME: f64 = 100.0;
const CHEVRON: f64 = 20.0;
const GRIP: f64 = 10.0;

#[derive(guinea::Mark)]
enum Marks {
    Table,
    Name,
    Chevron,
    Grip,
    Status,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Name,
    Chevron,
    Grip,
}

fn key(index: usize) -> u64 {
    100 + index as u64
}

/// The row and the part of it under `x`, `y`: only the rows in view take a
/// pointer, as on screen.
fn hit(rows: usize, top: usize, x: f64, y: f64) -> Option<(usize, Part)> {
    if y < 0.0 || y >= SHOWN as f64 * ROW {
        return None;
    }

    let index = top + (y / ROW) as usize;
    if index >= rows {
        return None;
    }

    let part = match x {
        x if x < 0.0 => return None,
        x if x < NAME => Part::Name,
        x if x < NAME + CHEVRON => Part::Chevron,
        x if x < NAME + CHEVRON + GRIP => Part::Grip,
        _ => return None,
    };

    Some((index, part))
}

fn row(index: usize, top: usize) -> Semantic {
    let y = (index as f64 - top as f64) * ROW;

    Semantic::new(Role::Row)
        .key(key(index))
        .bounds(Bounds::new(0.0, y, NAME + CHEVRON + GRIP, ROW))
        .children([
            Semantic::new(Role::Cell)
                .mark(Marks::Name)
                .text(SERVICES[index])
                .tip(format!("{} service", SERVICES[index]))
                .bounds(Bounds::new(0.0, y, NAME, ROW)),
            Semantic::new(Role::Button)
                .mark(Marks::Chevron)
                .bounds(Bounds::new(NAME, y, CHEVRON, ROW)),
            Semantic::new(Role::Button)
                .mark(Marks::Grip)
                .bounds(Bounds::new(NAME + CHEVRON, y, GRIP, ROW)),
        ])
}

fn table(rows: usize, top: Rc<Cell<usize>>) -> Semantic {
    let shown = top.clone();
    let scrolled = top;

    Semantic::new(Role::Table).mark(Marks::Table).items(Items::new(
        rows,
        move |index| row(index, shown.get()),
        move |index| {
            if index < scrolled.get() {
                scrolled.set(index);
            }
            if index >= scrolled.get() + SHOWN {
                scrolled.set(index + 1 - SHOWN);
            }
        },
    ))
}

/// A table of `rows` services painted on a composition host, three rows in
/// view: each row a name that selects it, a chevron that toggles it and a
/// grip that drags it.
pub struct TablePage {
    rows: usize,
    host: ElementRef<Grid>,
    top: Rc<Cell<usize>>,
    published: Option<Published>,
    pressed: Option<(u64, Part)>,
    selected: Option<u64>,
    toggled: Option<u64>,
    dropped: Option<(u64, usize)>,
}

impl Default for TablePage {
    fn default() -> Self {
        Self::with_rows(SERVICES.len())
    }
}

impl TablePage {
    fn with_rows(rows: usize) -> Self {
        let host = ElementRef::<Grid>::new();
        let top = Rc::new(Cell::new(0));

        let published = {
            let top = top.clone();
            publish(&host, move || table(rows, top.clone()))
        };

        Self {
            rows,
            host,
            top,
            published: Some(published),
            pressed: None,
            selected: None,
            toggled: None,
            dropped: None,
        }
    }

    fn hit(&self, info: &PointerEventInfo) -> Option<(usize, Part)> {
        hit(self.rows, self.top.get(), info.x, info.y)
    }
}

pub enum Tabling {
    Pressed(PointerEventInfo),
    Released(PointerEventInfo),
    Forget,
}

#[page]
impl Page for TablePage {
    type Params = usize;
    type Installs = ();
    type Message = Tabling;

    fn install(_ctx: &FeatureInitContext, _params: &usize) -> anyhow::Result<()> {
        Ok(())
    }

    fn init(_ctx: &FeatureInitContext, rows: &usize) -> Self {
        Self::with_rows(*rows)
    }

    fn update(&mut self, message: Tabling, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Tabling::Pressed(info) => {
                self.pressed = self.hit(&info).map(|(index, part)| (key(index), part));
            }
            Tabling::Released(info) => {
                let hit = self.hit(&info);
                match (self.pressed.take(), hit) {
                    (Some((dragged, Part::Grip)), Some((to, _))) => self.dropped = Some((dragged, to)),
                    (_, Some((index, Part::Name))) => self.selected = Some(key(index)),
                    (_, Some((index, Part::Chevron))) => self.toggled = Some(key(index)),
                    _ => {}
                }
            }
            Tabling::Forget => self.published = None,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                TextBlock::new().mark(Marks::Status).text(format!(
                    "selected {:?} toggled {:?} dropped {:?} top {}",
                    self.selected,
                    self.toggled,
                    self.dropped,
                    self.top.get()
                )),
                Border::new()
                    .on_pointer_pressed(cx.on(Tabling::Pressed))
                    .on_pointer_released(cx.on(Tabling::Released))
                    .content(Grid::new().element_ref(&self.host)),
            ))
            .into()
    }
}

fn status(page: &Mounted<'_, TablePage>) -> String {
    page.tree()
        .find(Marks::Status)
        .and_then(|status| status.text.clone())
        .unwrap_or_default()
}

#[guinea::test(iterations = 2)]
fn a_painted_table_is_read_where_its_element_is(h: &mut Harness) {
    let mut page = Mounted::<TablePage>::mount(h.segment(), 6).unwrap();
    assert!(page.find(Marks::Table).is_some(), "{:#?}", page.tree());

    assert_eq!(page.item_count(), SERVICES.len());

    let items = page.items();
    let names: Vec<Option<&str>> = items
        .iter()
        .map(|row| row.find(Marks::Name).and_then(|name| name.text.as_deref()))
        .collect();
    assert_eq!(names, SERVICES.map(Some));
    assert_eq!(items[1].kind, "Row");

    let themes = items[2].find(Marks::Name).unwrap();
    assert_eq!(themes.tip.as_deref(), Some("Themes service"));
    assert!(page.find_text("Themes").is_some(), "a row read once stays in the tree, as a realized item does");
}

#[guinea::test(iterations = 2)]
fn a_click_on_a_painted_row_lands_where_its_bounds_say(h: &mut Harness) {
    let mut page = Mounted::<TablePage>::mount(h.segment(), 6).unwrap();
    assert!(page.find(Marks::Table).is_some(), "{:#?}", page.tree());

    page.item_with_text("Spooler").click(Marks::Name).settle();
    page.settle();
    assert_eq!(status(&page), "selected Some(101) toggled None dropped None top 0");

    page.item(2).click(Marks::Chevron).settle();
    page.settle();
    assert_eq!(status(&page), "selected Some(101) toggled Some(102) dropped None top 0");

    page.item(0).click_here().settle();
    page.settle();
    assert_eq!(status(&page), "selected Some(100) toggled Some(102) dropped None top 0");
}

#[guinea::test(iterations = 2)]
fn a_row_out_of_view_is_scrolled_into_view_before_it_is_clicked(h: &mut Harness) {
    let mut page = Mounted::<TablePage>::mount(h.segment(), 6).unwrap();
    assert!(page.find(Marks::Table).is_some(), "{:#?}", page.tree());

    page.item(5).click(Marks::Name).settle();
    page.settle();
    assert_eq!(status(&page), "selected Some(105) toggled None dropped None top 3");
}

#[guinea::test(iterations = 2)]
fn a_drag_on_a_painted_grip_starts_inside_it(h: &mut Harness) {
    let mut page = Mounted::<TablePage>::mount(h.segment(), 6).unwrap();
    assert!(page.find(Marks::Table).is_some(), "{:#?}", page.tree());

    page.item(0).drag(Marks::Grip, Drag::by(0.0, 2.0 * ROW)).settle();
    page.settle();
    assert_eq!(status(&page), "selected None toggled None dropped Some((100, 2)) top 0");
}

#[guinea::test(iterations = 2)]
fn each_mounted_page_reads_the_tree_its_own_element_published(h: &mut Harness) {
    let mut wide = Mounted::<TablePage>::mount(h.segment(), 6).unwrap();
    let mut narrow = Mounted::<TablePage>::mount(h.child(), 4).unwrap();
    assert!(wide.find(Marks::Table).is_some(), "{:#?}", wide.tree());
    assert!(narrow.find(Marks::Table).is_some(), "{:#?}", narrow.tree());

    assert_eq!((wide.item_count(), narrow.item_count()), (6, 4));
}

#[guinea::test(iterations = 2)]
fn a_tree_goes_when_its_element_lets_it_go(h: &mut Harness) {
    let mut page = Mounted::<TablePage>::mount(h.segment(), 6).unwrap();
    assert!(page.find(Marks::Table).is_some(), "{:#?}", page.tree());

    page.send(Tabling::Forget);
    page.settle();
    assert!(page.find(Marks::Table).is_none(), "{:#?}", page.tree());
}
