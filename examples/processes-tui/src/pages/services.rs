use guinea::feature::FeatureInitContext;
use guinea::ratatui::{Handled, InputCx, Page, PageCx};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};

use processes_core::services::contracts::Services as Running;

use crate::cursor::Cursor;

#[derive(Default)]
pub struct Services {
    cursor: Cursor,
}

impl Page for Services {
    type Params = crate::routes::ServicesParams;

    type Installs = processes_core::services::ServicesFeature;

    fn install(
        ctx: &FeatureInitContext,
        _params: &Self::Params,
    ) -> anyhow::Result<Self::Installs> {
        ctx.install(&())
    }

    fn on_key(&mut self, cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        let (state, _) = cx.read::<Running>();
        let len = state.items.len();

        match key.code {
            KeyCode::Up => self.cursor.step(-1, len),
            KeyCode::Down => self.cursor.step(1, len),
            _ => return Handled::No,
        }

        Handled::Yes
    }

    fn render(&mut self, cx: &mut PageCx<'_, '_, Self>) {
        let (state, _) = cx.read::<Running>();
        let area = cx.area();

        let focused = self.cursor.row(state.items.len());
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
