use guinea::eframe::{Page, PageCx};
use guinea::feature::FeatureInitContext;

use processes_core::processes::contracts::{Kill, Processes as Running};
use processes_core::processes::pid_at;

#[derive(Default)]
pub struct Processes;

impl Page for Processes {
    type Params = crate::routes::ProcessesParams;

    type Installs = processes_core::processes::ProcessesFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (state, dispatch) = cx.read::<Running, _>();

        egui::ScrollArea::vertical().show(cx.ui(), |ui| {
            for (index, item) in state.items.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(item);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Read at click time rather than captured: the actor
                        // refreshes the list, and this row may not be the row
                        // it was when the frame started.
                        if ui.button("Kill").clicked()
                            && let Some(pid) = pid_at(&state.items, index)
                        {
                            dispatch.emit(Kill(pid));
                        }
                    });
                });
            }
        });
    }
}
