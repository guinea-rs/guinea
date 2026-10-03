use guinea::eframe::{Layout, LayoutCx};
use guinea::feature::FeatureInitContext;
use guinea_plugin_l10n::{L10nAccess, Localization};

use processes_core::l10n::L10n;
use processes_core::tabs::contracts::Tabs;

use crate::pages::metrics::Metrics;
use crate::pages::processes::Processes;
use crate::pages::services::Services;
use crate::routes::Route;

#[derive(Default)]
pub struct TabsLayout;

impl Layout for TabsLayout {
    type Params = crate::routes::TabsLayoutParams;

    type Installs = processes_core::tabs::TabsFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let (state, _) = cx.state::<Tabs, _>();
        let strings = &cx.l10n::<L10n>();
        let switch = cx.language_switch::<L10n>();
        let nav = cx.navigate::<Route>();

        // Which tab is current comes from the chain, not from a copy of the
        // route in state - the router already knows what it mounted.
        let current = [
            cx.child_is::<Processes>(),
            cx.child_is::<Services>(),
            cx.child_is::<Metrics>(),
        ];

        // Taken before the borrow below: drawing the page is the layout's
        // job, and it needs the same `ui`.
        let page = cx.outlet();

        let ui = cx.ui();
        ui.horizontal(|ui| {
            ui.strong(strings.app_title());
            ui.separator();

            // What this layout was reached with, not one invented here:
            // `routes!` derived it from the pages below.
            let context = || state.context.clone();
            if ui.selectable_label(current[0], "Processes").clicked() {
                nav.to(Route::Processes { context: context() });
            }
            if ui.selectable_label(current[1], "Services").clicked() {
                nav.to(Route::Services { context: context() });
            }
            if ui.selectable_label(current[2], "Metrics").clicked() {
                nav.to(Route::Metrics { context: context() });
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(language_label(strings)).clicked() {
                    switch.to(next_language(strings));
                }
            });
        });
        ui.separator();

        // The rest of the frame is the page's, minus the status line below.
        let status = ui.text_style_height(&egui::TextStyle::Body) + 12.0;
        let room = egui::vec2(ui.available_width(), ui.available_height() - status);
        ui.allocate_ui(room, |ui| page.draw(ui));

        ui.separator();
        ui.label(status_line(strings, &state));
    }
}

fn language_label(strings: &L10n) -> &'static str {
    if strings.tag() == "ru" {
        "English"
    } else {
        "Русский"
    }
}

fn next_language(strings: &L10n) -> &'static str {
    if strings.tag() == "ru" { "en" } else { "ru" }
}

fn status_line(strings: &L10n, state: &Tabs) -> String {
    let killed = state
        .last_killed
        .as_deref()
        .map(|name| format!(" · {}", strings.process_killed_toast(name.to_string())))
        .unwrap_or_default();

    format!(
        "kills here {} · everywhere {}{killed}",
        state.kills_this_window, state.kills_all_windows,
    )
}
