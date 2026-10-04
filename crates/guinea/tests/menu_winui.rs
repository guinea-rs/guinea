#![cfg(all(windows, feature = "winui"))]

//! A layout's menu built from the tree: its children, as routes, labelled
//! by an exhaustive `match`.

use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, layout, page};
use windows_reactor::{StackPanel, TextBlock, View};

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    type Params = ShellParams;

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let items = cx
            .child_routes::<MenuRoute>()
            .into_iter()
            .map(|child| {
                let label = match child.route {
                    MenuRoute::Processes {} => "Processes",
                    MenuRoute::Services {} => "Services",
                };
                let marker = if child.current { "> " } else { "" };
                TextBlock::new().text(format!("{marker}{label}")).into()
            })
            .collect::<Vec<View>>();

        StackPanel::new()
            .children((StackPanel::new().children(items), cx.outlet()))
            .into()
    }
}

#[derive(Default)]
pub struct Processes;

#[page]
impl Page for Processes {
    type Params = ProcessesParams;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("processes").into()
    }
}

#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    type Params = ServicesParams;

    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("services").into()
    }
}

guinea::routes! {
    MenuRoute {
        layout(Shell) {
            page(Processes) { }
            page(Services) { }
        }
    }
}

#[guinea::test(iterations = 2)]
fn a_layout_lists_its_children_and_marks_the_current_one(h: &mut Harness) {
    let mut app = Mounted::routed(h, MenuRoute::Services {}).unwrap();
    app.settle();

    assert!(app.find_text("Processes").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("> Services").is_some(), "{:#?}", app.tree());

    app.navigate(MenuRoute::Processes {});
    app.settle();

    assert!(app.find_text("> Processes").is_some(), "{:#?}", app.tree());
    assert!(app.find_text("Services").is_some(), "{:#?}", app.tree());
}
