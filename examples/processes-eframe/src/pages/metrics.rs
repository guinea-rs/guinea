use guinea::eframe::{Page, PageCx};
use guinea::feature::FeatureInitContext;
use guinea_widgets::chart::RingSeries;

use processes_core::metrics::contracts::Metrics as Sampling;

#[derive(Default)]
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

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (state, _) = cx.read::<Sampling>();
        let cpu = values(&state.cpu);
        let memory = values(&state.memory);

        let ui = cx.ui();
        gauge(ui, "CPU", latest(&cpu));
        gauge(ui, "RAM", latest(&memory));

        ui.add_space(8.0);
        // The same RingSeries the WinUI chart draws and the terminal turns
        // into a sparkline - here, bars painted by hand, because egui has no
        // chart of its own.
        history(ui, &cpu);
    }
}

fn gauge(ui: &mut egui::Ui, label: &str, value: f32) {
    ui.horizontal(|ui| {
        ui.strong(format!("{label} {:.0}%", value));
        ui.add(egui::ProgressBar::new(value / 100.0).desired_width(220.0));
    });
}

fn history(ui: &mut egui::Ui, samples: &[f32]) {
    let height = ui.available_height().max(40.0);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    let painter = ui.painter_at(rect);
    let width = 4.0;
    let gap = 1.0;
    let fits = ((rect.width() / (width + gap)) as usize).max(1);
    let shown = samples.len().saturating_sub(fits);

    for (column, sample) in samples[shown..].iter().enumerate() {
        let bar = (sample / 100.0).clamp(0.0, 1.0) * rect.height();
        let x = rect.right() - (column as f32 + 1.0) * (width + gap);

        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x, rect.bottom() - bar),
                egui::vec2(width, bar),
            ),
            0.0,
            egui::Color32::from_rgb(0x4a, 0x90, 0xd9),
        );
    }
}

fn values(series: &RingSeries) -> Vec<f32> {
    let (older, newer) = series.as_slices();
    older.iter().chain(newer).map(|(_, value)| *value).collect()
}

fn latest(values: &[f32]) -> f32 {
    values.last().copied().unwrap_or(0.0)
}
