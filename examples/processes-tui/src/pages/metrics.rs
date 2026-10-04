use guinea::feature::FeatureInitContext;
use guinea::ratatui::{Page, PageCx};
use ratatui::layout::{Constraint, Direction, Layout as Rows};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Sparkline};

use processes_core::metrics::contracts::Metrics as Sampling;

pub struct Metrics;

impl Page for Metrics {
    type Params = crate::routes::MetricsParams;

    type Installs = processes_core::metrics::MetricsFeature;

    fn install(
        ctx: &FeatureInitContext,
        _params: &Self::Params,
    ) -> anyhow::Result<Self::Installs> {
        ctx.install(&())
    }

    fn render(cx: &mut PageCx<'_, '_, Self>) {
        let (state, _) = cx.read::<Sampling>();

        let rows = Rows::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(cx.area());

        // A sparkline rather than the WinUI line chart: same `RingSeries`
        // underneath, drawn with what a terminal has. Percentages are already
        // 0..100, which is the resolution a row of blocks can show anyway.
        let sampled = |ring: &guinea_widgets::chart::RingSeries| -> Vec<u64> {
            ring.as_points().into_iter().map(|(_, v)| v as u64).collect()
        };
        let cpu = sampled(&state.cpu);
        let memory = sampled(&state.memory);

        let latest = |series: &[u64]| series.last().copied().unwrap_or(0);

        cx.frame().render_widget(
            Sparkline::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" CPU {}% ", latest(&cpu)))
                        .title_style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .data(&cpu)
                .max(100)
                .style(Style::default().fg(Color::Blue)),
            rows[0],
        );

        cx.frame().render_widget(
            Sparkline::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" RAM {}% ", latest(&memory)))
                        .title_style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .data(&memory)
                .max(100)
                .style(Style::default().fg(Color::Green)),
            rows[1],
        );
    }
}
