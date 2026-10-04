use guinea::feature::FeatureInitContext;
use guinea::winui::{Layout, LayoutCx, UseNavigate, UseRouteChange, Window, layout, window};
use guinea_plugin_l10n::{L10nAccess, Localization};
use windows_reactor::{Button, Orientation, StackPanel, TextBlock, View};

use crate::routes::Route;
use processes_core::l10n::L10n;

use processes_core::tabs::contracts::Tabs;

#[derive(Default)]
pub struct TabsLayout;

#[layout]
impl Layout for TabsLayout {
    type Params = crate::routes::TabsLayoutParams;

    type Installs = processes_core::tabs::TabsFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let (tabs, _) = cx.read::<Tabs>();
        cx.use_route_change(|from, to| tracing::debug!(?from, to, "route"));

        let nav = cx.use_navigate::<Route>();
        let l10n = cx.l10n::<L10n>();
        let switch = cx.language_switch::<L10n>();

        let is_russian = l10n.tag() == "ru";
        let lang_button_label = if is_russian { "English" } else { "Русский" };

        // What this layout was reached with, not one invented here: `routes!`
        // derived it from the pages below.
        let context = || tabs.context.clone();

        // A window is opened from `update`, never from here - so what a click
        // carries is the ask, and the segment does the opening.
        let second = cx.open_window(window(
            Window::new()
                .title("guinea · processes (2)")
                .client_size(420.0, 420.0),
            Route::Processes { context: context() },
        ));

        let tab_bar = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                Button::new()
                    .on_click(nav.to_handler(Route::Processes {
                        context: context(),
                    }))
                    .content(TextBlock::new().text("Processes")),
                Button::new()
                    .on_click(nav.to_handler(Route::Services {
                        context: context(),
                    }))
                    .content(TextBlock::new().text("Services")),
                Button::new()
                    .on_click(nav.to_handler(Route::Metrics {
                        context: context(),
                    }))
                    .content(TextBlock::new().text("Metrics")),
                Button::new()
                    .on_click(second)
                    .content(TextBlock::new().text("Open window")),
                // The language is the application's: switching it here redraws
                // every open window, not just this one - open a second window
                // and flip the language here.
                Button::new()
                    .on_click(move || {
                        let next = if is_russian { "en" } else { "ru" };
                        switch.to(next);
                    })
                    .content(TextBlock::new().text(lang_button_label)),
            ));

        let toast = tabs
            .last_killed
            .clone()
            .map(|name| l10n.process_killed_toast(name))
            .unwrap_or_default();

        StackPanel::new()
            .spacing(8.0)
            .children((
                TextBlock::new().text(l10n.app_title()),
                tab_bar,
                TextBlock::new().text(format!("shell installed {}x", tabs.install_count)),
                TextBlock::new().text(format!("kills (this window): {}", tabs.kills_this_window)),
                TextBlock::new().text(format!("kills (all windows): {}", tabs.kills_all_windows)),
                TextBlock::new().text(toast),
                cx.outlet(),
            ))
            .into()
    }
}
