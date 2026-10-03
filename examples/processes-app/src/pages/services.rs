use guinea::feature::FeatureInitContext;
use guinea::winui::{Page, PageCx, page};
use windows_reactor::{ChildrenControl, Orientation, StackPanel, TextBlock, View};

use processes_core::services::contracts::Services as Running;

#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    type Params = crate::routes::ServicesParams;

    type Installs = processes_core::services::ServicesFeature;

    fn install(ctx: &FeatureInitContext, _params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(&())
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        let (state, _dispatch) = cx.read::<Running, _>();

        let rows: Vec<(String, View)> = state
            .items
            .iter()
            .map(|row| (row.clone(), TextBlock::new().text(row.clone()).into()))
            .collect();

        StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(16.0)
            .children((
                TextBlock::new().text("Services"),
                StackPanel::new()
                    .spacing(6.0)
                    .children((View::keyed_fragment(rows),)),
            ))
    }
}
