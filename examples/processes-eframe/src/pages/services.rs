use guinea::eframe::{Page, PageCx};
use guinea::feature::FeatureInitContext;

use processes_core::services::contracts::Services as Running;

#[derive(Default)]
pub struct Services;

impl Page for Services {
    type Params = crate::routes::ServicesParams;

    type Installs = processes_core::services::ServicesFeature;

    fn install(
        ctx: &FeatureInitContext,
        _params: &Self::Params,
    ) -> anyhow::Result<Self::Installs> {
        ctx.install(&())
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (state, _) = cx.read::<Running, _>();

        egui::ScrollArea::vertical().show(cx.ui(), |ui| {
            for item in &state.items {
                ui.label(item);
            }
        });
    }
}
