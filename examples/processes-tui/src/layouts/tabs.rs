use guinea::feature::FeatureInitContext;
use guinea::ratatui::{Layout, LayoutCx};
use ratatui::layout::{Constraint, Direction, Layout as Rows};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use guinea_plugin_l10n::L10nAccess;
use processes_core::l10n::L10n;
use processes_core::tabs::contracts::Tabs;

use crate::pages::metrics::Metrics;
use crate::pages::processes::Processes;
use crate::pages::services::Services;

pub struct TabsLayout;

impl Layout for TabsLayout {
    type Params = crate::routes::TabsLayoutParams;

    type Installs = processes_core::tabs::TabsFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn render(cx: &mut LayoutCx<'_, '_, Self>) {
        let (state, _) = cx.state::<Tabs, _>();
        let strings = cx.l10n::<L10n>();

        let rows = Rows::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(2),
            ])
            .split(cx.area());

        let current = [
            cx.child_is::<Processes>(),
            cx.child_is::<Services>(),
            cx.child_is::<Metrics>(),
        ];
        let names = ["processes", "services", "metrics"];
        let mut spans = vec![Span::raw(format!(" {} · ", strings.app_title()))];
        spans.extend(names.iter().zip(current).enumerate().flat_map(
            |(i, (name, selected))| {
                let style = if selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                [
                    Span::styled(format!(" {} {name} ", i + 1), style),
                    Span::raw(" "),
                ]
            },
        ));
        let tabs = Line::from(spans);
        cx.frame().render_widget(Paragraph::new(tabs), rows[0]);

        cx.outlet(rows[1]);

        let killed = state
            .last_killed
            .as_deref()
            .map(|name| format!(" · {}", strings.process_killed_toast(name.to_string())))
            .unwrap_or_default();
        let status = Paragraph::new(format!(
            "kills here {} · everywhere {}{killed}\n1/2/3 switch · ↑/↓ focus · k kill · l language · esc back · q quit",
            state.kills_this_window, state.kills_all_windows,
        ))
        .block(Block::default().borders(Borders::TOP));
        cx.frame().render_widget(status, rows[2]);
    }
}
