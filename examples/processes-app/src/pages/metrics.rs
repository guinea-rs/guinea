//! The metrics page. Like `processes`, it owns something now: the chart.
//!
//! A chart that redraws only when its data grew has to remember what it last
//! drew, and the hover readout has to survive the pointer standing still while
//! the series tick underneath it. Both are state, and a page is where state
//! goes.

use guinea::feature::FeatureInitContext;
use guinea::winui::{Page, PageCx, UpdateCx, page};
use guinea_widgets::chart::{Chart, HoverInfo, Interpolation, LineChartOptions, Series};
use guinea_widgets::color::{hex, hex_alpha};
use windows_canvas::ColorF;
use windows_reactor::{
    Border, ChildrenControl, ContentControl, LayoutControl, Orientation, StackPanel, TextBlock,
    View,
};

use processes_core::metrics::contracts::Metrics as Sampling;

#[derive(Default)]
pub struct Metrics {
    chart: Chart,
}

/// The pointer moved over the chart.
///
/// It carries nothing on purpose. What is under the pointer is read from the
/// chart at view time, against whatever the series say *now* - a value carried
/// in the message would be the value at the moment of the move, and would then
/// sit there while the data ticked underneath it.
pub enum Msg {
    Hovered,
}

const CPU_COLOR: ColorF = hex(0x3a82f5);
const MEMORY_COLOR: ColorF = hex(0xf59e0a);

#[page]
impl Page for Metrics {
    type Params = crate::routes::MetricsParams;

    type Installs = processes_core::metrics::MetricsFeature;

    type Message = Msg;

    fn install(ctx: &FeatureInitContext, _params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(&())
    }

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            // Nothing to change: the chart already recorded where the pointer
            // is. What this does is bring the page round to draw again, which
            // is what a message is for.
            Msg::Hovered => {}
        }
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        let (state, _) = cx.read::<Sampling>();

        self.chart.publish(
            vec![
                Series {
                    color: CPU_COLOR,
                    interpolation: Interpolation::Smooth,
                    fill: Some(hex_alpha(0x3a82f5, 38)),
                    points: state.cpu.as_points(),
                },
                Series {
                    color: MEMORY_COLOR,
                    interpolation: Interpolation::Linear,
                    fill: None,
                    points: state.memory.as_points(),
                },
            ],
            LineChartOptions::default(),
        );

        // The chart hands back an erased view, and size is a capability of the
        // widget carrying it - so the size goes on the wrapper.
        let chart = Border::new()
            .width(380.0)
            .height(220.0)
            .content(self.chart.view(cx.on(|_: Option<HoverInfo>| Msg::Hovered)));

        // Read here rather than carried in the message: the series tick on
        // their own, so a pointer resting on the chart follows them instead of
        // reporting whatever was under it when it last moved.
        let readout = self
            .chart
            .hovered()
            .map(|h| {
                format!(
                    "CPU {:.0}% · RAM {:.0}%",
                    h.values[0],
                    h.values.get(1).copied().unwrap_or(0.0)
                )
            })
            .unwrap_or_else(|| "hover the chart for values".to_string());

        StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(12.0)
            .children((
                TextBlock::new().text("Metrics"),
                TextBlock::new().text(readout),
                chart,
            ))
    }
}
