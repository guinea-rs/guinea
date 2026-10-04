use guinea::core::feature::Bound;
use guinea::feature::FeatureInitContext;
use guinea::ratatui::{Page, PageCx};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};

use processes_core::processes::contracts::{Listed, Processes as Running};

use crate::cursor::{Cursor, Move};

pub struct Processes;

impl Page for Processes {
    type Params = crate::routes::ProcessesParams;

    /// The feature, and the focus this page keeps of its own - both, because
    /// both are read below and a claim that goes undeclared is a claim the
    /// page cannot read.
    type Installs = (processes_core::processes::ProcessesFeature, Bound<Cursor>);

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        let cursor = ctx.state::<Cursor>().plain();
        let listing = ctx.install(params.context.as_str())?;

        // The focus points into a list the actor owns, so it has to hear when
        // that list is replaced - killing the last row leaves the focus past
        // the end otherwise.
        let observing = cursor.clone();
        ctx.observe::<Running>(move |update| {
            let Listed::Items(items) = update;
            observing.push(Move {
                delta: 0,
                len: items.len(),
            });
        });

        Ok((listing, cursor))
    }

    fn render(cx: &mut PageCx<'_, '_, Self>) {
        let (state, _) = cx.read::<Running>();
        let (cursor, _) = cx.read::<Cursor>();
        let area = cx.area();

        let focused = cursor.row;
        let items: Vec<ListItem> = state
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let row = ListItem::new(format!(" {} {item}", i + 1));
                if i == focused {
                    row.style(Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    row
                }
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Processes ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        );
        cx.frame().render_widget(list, area);
    }
}
