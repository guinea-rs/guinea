use guinea::core::feature::Bound;
use guinea::feature::FeatureInitContext;
use guinea::ratatui::{Page, PageCx};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};

use processes_core::services::contracts::{Listed, Services as Running};

use crate::cursor::{Cursor, Move};

pub struct Services;

impl Page for Services {
    type Params = crate::routes::ServicesParams;

    /// The feature, and the focus this page keeps of its own - both, because
    /// both are read below.
    type Installs = (processes_core::services::ServicesFeature, Bound<Cursor>);

    fn install(
        ctx: &FeatureInitContext,
        _params: &Self::Params,
    ) -> anyhow::Result<Self::Installs> {
        let cursor = ctx.state::<Cursor>().plain();
        let catalogue = ctx.install(&())?;

        let observing = cursor.clone();
        ctx.observe::<Running>(move |update| {
            let Listed::Items(items) = update;
            observing.push(Move {
                delta: 0,
                len: items.len(),
            });
        });

        Ok((catalogue, cursor))
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
                let row = ListItem::new(format!(" {item}"));
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
                .title(" Services ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        );
        cx.frame().render_widget(list, area);
    }
}
