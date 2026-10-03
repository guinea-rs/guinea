//! The one place two message types meet.
//!
//! `cx.mine(..)` seals this layout's own widgets; `cx.outlet()` hands over the
//! page's, already sealed. Neither side names the other's message type, and
//! adding a fourth page changes nothing here except a tab button.

use guinea::feature::FeatureInitContext;
use guinea::iced::{Element, Envelope, Layout, LayoutCx, UpdateCx, layout};
use guinea_plugin_l10n::{L10nAccess, Language, Localization, SwitchLanguage};
use iced::Length::Fill;
use iced::widget::{button, column, container, row, space, text};

use processes_core::l10n::L10n;
use processes_core::tabs::TabsFeature;
use processes_core::tabs::contracts::Tabs;

use crate::pages::draft::Draft;
use crate::pages::login::Login;
use crate::pages::metrics::Metrics;
use crate::pages::processes::Processes;
use crate::pages::services::Services;
use crate::routes::Route;

/// Chrome only - what it shows comes from the reducer and from the chain, so
/// there is nothing to keep here.
#[derive(Default)]
pub struct TabsLayout;

#[derive(Clone, Copy)]
pub enum Tab {
    Processes,
    Services,
    Metrics,
    Login,
    Draft,
}

#[derive(Clone)]
pub enum Chrome {
    Show(Tab),
    ToggleLanguage,
}

#[layout]
impl Layout for TabsLayout {
    type Params = crate::routes::TabsLayoutParams;
    type Message = Chrome;
    type Installs = TabsFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<TabsFeature> {
        ctx.install::<TabsFeature>(&params.context)
    }

    fn update(&mut self, message: Chrome, cx: &mut UpdateCx<'_, Self>) {
        match message {
            Chrome::Show(tab) => {
                // The context this layout was reached with, not one invented
                // here: `routes!` derived it from the pages below.
                let (tabs, _) = cx.read::<Tabs, _>();
                let context = tabs.context.clone();
                cx.navigate::<Route>().to(match tab {
                    Tab::Processes => Route::Processes { context },
                    Tab::Services => Route::Services { context },
                    Tab::Metrics => Route::Metrics { context },
                    Tab::Login => Route::Login { context },
                    Tab::Draft => Route::Draft { context },
                });
            }
            Chrome::ToggleLanguage => {
                let (language, switch) = cx.read::<Language<L10n>, _>();
                switch.emit(SwitchLanguage(next_language(language.strings()).into()));
            }
        }
    }

    fn view<'a>(&'a self, cx: &mut LayoutCx<'a, Self>) -> Element<'a, Envelope> {
        let strings = &cx.l10n::<L10n>();
        let (tabs, _) = cx.read::<Tabs, _>();

        // Which tab is current comes from the chain, not from a copy of the
        // route in state - the router already knows what it mounted.
        let bar = row![
            text(strings.app_title().to_string()).size(18),
            tab("Processes", Tab::Processes, cx.child_is::<Processes>()),
            tab("Services", Tab::Services, cx.child_is::<Services>()),
            tab("Metrics", Tab::Metrics, cx.child_is::<Metrics>()),
            tab("Login", Tab::Login, cx.child_is::<Login>()),
            tab("Draft", Tab::Draft, cx.child_is::<Draft>()),
            space().width(Fill),
            button(text(language_label(strings))).on_press(Chrome::ToggleLanguage),
        ]
        .spacing(8)
        .padding(8);

        column![
            cx.mine(bar),
            container(cx.outlet()).height(Fill),
            cx.mine(container(text(status_line(strings, &tabs))).padding(8)),
        ]
        .into()
    }
}

fn tab(label: &'static str, tab: Tab, current: bool) -> iced::widget::Button<'static, Chrome> {
    button(text(label))
        .style(if current {
            button::primary
        } else {
            button::text
        })
        .on_press(Chrome::Show(tab))
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
