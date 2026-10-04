use std::rc::Rc;

use guinea::feature::FeatureInitContext;
use guinea::slint::{Layout, LayoutCx, ToSlint};
use guinea_plugin_l10n::{Language, Localization, SwitchLanguage};
use slint::ComponentHandle;

use processes_core::l10n::L10n;
use processes_core::tabs::contracts::Tabs;

use crate::routes::Route;
use crate::ui::{AppWindow, TabsModel};

pub struct TabsLayout;

impl Layout for TabsLayout {
    type Params = crate::routes::TabsLayoutParams;

    type Installs = processes_core::tabs::TabsFeature;

    fn install(ctx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self::Installs> {
        ctx.install(params.context.as_str())
    }

    fn bind(cx: LayoutCx<Self>) {
        let root = cx.root::<AppWindow>();
        let model = root.global::<TabsModel>();

        let binding = cx.binding::<Tabs>();
        let language = cx.binding::<Language<L10n>>();
        let refresh: Rc<dyn Fn()> = {
            let root = root.clone_strong();
            let binding = binding.clone();
            let language = language.clone();
            Rc::new(move || {
                let model = root.global::<TabsModel>();
                let language = language.get();
                let strings = language.strings();
                model.set_app_title(strings.app_title().to_slint());
                model.set_language_label(language_label(strings).to_slint());
                model.set_status(status_line(strings, &binding.peek()).to_slint());
            })
        };

        cx.bind::<Tabs, _>({
            let refresh = refresh.clone();
            move |_| refresh()
        });

        cx.bind::<Language<L10n>, _>({
            let refresh = refresh.clone();
            move |_| refresh()
        });

        let nav = cx.navigate::<Route>();
        let context_of = binding.clone();
        model.on_go(move |index| {
            // What this layout was reached with, not one invented here:
            // `routes!` derived it from the pages below.
            let context = context_of.peek().context.clone();
            match index {
                0 => nav.to(Route::Processes { context }),
                1 => nav.to(Route::Services { context }),
                _ => nav.to(Route::Metrics { context }),
            }
        });

        model.on_toggle_language(move || {
            let next = if language.peek().strings().tag() == "ru" {
                "en"
            } else {
                "ru"
            };
            language.dispatch().emit(SwitchLanguage(next.into()));
        });
    }
}

fn language_label(strings: &L10n) -> &'static str {
    if strings.tag() == "ru" {
        "English"
    } else {
        "Русский"
    }
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
