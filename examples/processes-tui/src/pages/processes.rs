use guinea::feature::FeatureInitContext;
use guinea::ratatui::{Handled, InputCx, Page, PageCx};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};

use processes_core::processes::contracts::{Kill, Processes as Running};

use crate::cursor::Cursor;

#[derive(Default)]
pub struct Processes {
    cursor: Cursor,
}

impl Page for Processes {
    type Params = crate::routes::ProcessesParams;

    type Installs = processes_core::processes::ProcessesFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn on_key(&mut self, cx: &mut InputCx<'_, Self>, key: &KeyEvent) -> Handled {
        let (state, running) = cx.read::<Running>();
        let len = state.items.len();

        match key.code {
            KeyCode::Up => self.cursor.step(-1, len),
            KeyCode::Down => self.cursor.step(1, len),
            KeyCode::Char('k') => {
                let focused = self.cursor.row(len);
                if let Some(pid) = processes_core::processes::pid_at(&state.items, focused) {
                    running.emit(Kill(pid));
                }
            }
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
