#![cfg(all(windows, feature = "winui"))]

//! A layout's menu built from its route enum: every place, labelled by an
//! exhaustive `match`, the current one marked.

use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, Route, layout, page};
use windows_reactor::{StackPanel, TextBlock, View};

#[derive(Clone, PartialEq, Debug, Route)]
pub enum Menu {
    Processes,
    Services,
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Menu>();
        let current = nav.current();

        let items = Menu::samples()
            .into_iter()
            .flatten()
            .map(|place| {
                let label = match place {
                    Menu::Processes => "Processes",
                    Menu::Services => "Services",
                };
                let marker = if current.as_ref() == Some(&place) { "> " } else { "" };
                TextBlock::new().text(format!("{marker}{label}")).into()
            })
            .collect::<Vec<View>>();

        let page = cx.outlet(&nav, |place, cx| match place {
            Menu::Processes => cx.mount::<Processes>(&()),
            Menu::Services => cx.mount::<Services>(&()),
        });

        StackPanel::new()
            .children((StackPanel::new().children(items), page))
            .into()
    }
}

#[derive(Default)]
pub struct Processes;

#[page]
impl Page for Processes {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("processes").into()
    }
}

#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("services").into()
    }
}

#[guinea::test(iterations = 2)]
fn a_layout_lists_its_places_and_marks_the_current_one(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Menu::Services).unwrap();
    app.settle();

    assert!(app.find_text("Processes").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("> Services").is_some(), "{:#?}", app.tree());

    app.navigate(Menu::Processes);
    app.settle();

    assert!(app.find_text("> Processes").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("Services").is_some(), "{:#?}", app.tree());
}
