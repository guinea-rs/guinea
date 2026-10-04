//! The page that keeps its state while it is not mounted.
//!
//! `CACHE_STATE_IN_MEMORY` covers the whole segment, this node and the
//! domain's reducers alike: the sampled history belongs to a reducer an actor
//! fills, and without the cache the graph starts over every time the tab is
//! left and comes back.

use guinea::feature::FeatureInitContext;
use guinea::iced::{Element, Page, PageCx, page};
use guinea_widgets::chart::RingSeries;
use iced::widget::{column, progress_bar, row, text};

use processes_core::metrics::MetricsFeature;
use processes_core::metrics::contracts::Metrics as Sampling;

#[derive(Default)]
pub struct Metrics;

/// What this page can say. Nothing - `#[page]` named the type, and this is the
/// one place the helpers below have to name it back.
type Quiet = <Metrics as Page>::Message;

#[page]
impl Page for Metrics {
    const CACHE_STATE_IN_MEMORY: bool = true;

    type Params = crate::routes::MetricsParams;
    type Installs = MetricsFeature;

    fn install(ctx: &FeatureInitContext, _params: &Self::Params) -> anyhow::Result<MetricsFeature> {
        ctx.install::<MetricsFeature>(&())
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> Element<'_, Quiet> {
        let (metrics, _) = cx.read::<Sampling>();
        let cpu = values(&metrics.cpu);
        let memory = values(&metrics.memory);

        column![
            gauge("CPU", &cpu),
            gauge("RAM", &memory),
            // The same `RingSeries` the WinUI chart draws and the terminal
            // turns into a sparkline, here as a row of bars - iced has no
            // chart of its own either.
            history(&cpu),
        ]
        .spacing(12)
        .padding(12)
        .into()
    }
}

fn values(series: &RingSeries) -> Vec<f32> {
    series.as_points().into_iter().map(|(_, v)| v).collect()
}

fn latest(samples: &[f32]) -> f32 {
    samples.last().copied().unwrap_or(0.0)
}

// `'static`: these build from the numbers rather than borrowing them, so
// nothing here outlives the call - unlike `Draft`, which is the page that
// needs the borrow a view is now allowed to take.
fn gauge(label: &str, samples: &[f32]) -> Element<'static, Quiet> {
    let value = latest(samples);
    row![
        text(format!("{label} {value:.0}%")).width(90),
        progress_bar(0.0..=100.0, value),
    ]
    .spacing(8)
    .into()
}

fn history(samples: &[f32]) -> Element<'static, Quiet> {
    let bars = samples
        .iter()
        .rev()
        .take(60)
        .rev()
        .map(|value| {
            progress_bar(0.0..=100.0, *value)
                .vertical()
                .length(80)
                .girth(4)
                .into()
        })
        .collect::<Vec<Element<'static, Quiet>>>();

    row(bars).spacing(2).into()
}
