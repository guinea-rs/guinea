//! The process list, and the one page that owns something of its own.
//!
//! Its column widths. They used to live inside the table, in a hook slot with
//! an `Rc<Cell>` per column that the drag wrote to - which meant two tables of
//! the same shape shared a slot or did not depending on where the slot landed,
//! and that nothing could save them. A page is an Elm node now, so they are a
//! field, a drag is a message, and `update` is the one place they change.

use guinea::feature::FeatureInitContext;
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};
use guinea_widgets::table::{ColumnSpec, ColumnWidths, Resized, table};
use windows_reactor::{Button, Orientation, StackPanel, TextBlock, View};

use processes_core::processes::contracts::{Kill, Processes as Running};

#[derive(Default)]
pub struct Processes {
    widths: ColumnWidths,
    /// Which row is selected.
    ///
    /// Here rather than left to the list: the page redraws whenever the
    /// process list ticks, and a selection the list held by itself would be
    /// re-declared away on the next publication - it would light up and drop
    /// again immediately.
    selected: Option<usize>,
}

pub enum Msg {
    Resized(Resized),
    Selected(Option<usize>),
}

#[derive(guinea::Mark, Clone, Copy, PartialEq)]
enum Column {
    Name,
    Actions,
}

#[derive(guinea::Mark)]
enum Marks {
    Kill,
}

struct Row {
    pid: u32,
    label: String,
}

#[page]
impl Page for Processes {
    type Params = crate::routes::ProcessesParams;

    type Installs = processes_core::processes::ProcessesFeature;

    type Message = Msg;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::Resized(drag) => self.widths.apply(drag),
            Msg::Selected(row) => self.selected = row,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (state, dispatch) = cx.read::<Running>();

        let rows: Vec<Row> = state
            .items
            .iter()
            .map(|item| Row {
                pid: parse_pid(item),
                label: item.clone(),
            })
            .collect();

        let columns = vec![
            ColumnSpec::new(Column::Name, "Process", 280.0, |row: &Row| {
                TextBlock::new().text(row.label.clone()).into()
            }),
            ColumnSpec::new(Column::Actions, "", 80.0, {
                let dispatch = dispatch.clone();
                move |row: &Row| {
                    let pid = row.pid;
                    let dispatch = dispatch.clone();
                    Button::new()
                        .mark(Marks::Kill)
                        .on_click(move || dispatch.emit(Kill(pid)))
                        .content(TextBlock::new().text("Kill"))
                        .into()
                }
            }),
        ];

        StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(16.0)
            .children((
                TextBlock::new().text("Processes"),
                table(rows, columns)
                    .widths(&self.widths)
                    .on_resize(cx.on(Msg::Resized))
                    .selection(self.selected, cx.on(Msg::Selected))
                    .build(),
            ))
            .into()
    }
}

fn parse_pid(row: &str) -> u32 {
    row.rsplit_once("(pid ")
        .and_then(|(_, rest)| rest.trim_end_matches(')').trim().parse().ok())
        .unwrap_or(0)
}
