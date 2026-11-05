#![cfg(all(windows, feature = "winui"))]
use guinea::app::Harness;
use guinea::winui::harness::Mounted;
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, Window, page};
use windows_reactor::{Border, Button, StackPanel, TextBlock, View};
mod sidebar {
    #[derive(guinea::Mark)]
    pub enum Marks {
        Memory,
    }
}
mod performance {
    #[derive(guinea::Mark)]
    pub enum Marks {
        Memory,
    }
}
#[derive(guinea::Mark)]
enum Repeated {
    Run,
    Missing,
}
#[derive(guinea::Mark)]
enum Containers {
    Left,
    Right,
}
#[derive(Clone, Debug, PartialEq, guinea::Routable)]
enum Route {
    Home,
}
#[derive(Default)]
struct Fixture {
    count: u32,
}
#[page]
impl Page for Fixture {
    type Message = ();
    fn update(&mut self, _: (), _: &mut UpdateCx<'_, Self>) {
        self.count += 1;
    }
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        StackPanel::new()
            .children((
                Border::new()
                    .mark(sidebar::Marks::Memory)
                    .content("Sidebar memory"),
                Button::new()
                    .mark(performance::Marks::Memory)
                    .content("Page memory")
                    .on_click(cx.on(|()| ())),
                Border::new().mark(Containers::Left).content(
                    Button::new()
                        .mark(Repeated::Run)
                        .content("Left")
                        .on_click(cx.on(|()| ())),
                ),
                Border::new().mark(Containers::Right).content(
                    Button::new()
                        .mark(Repeated::Run)
                        .content("Right")
                        .on_click(cx.on(|()| ())),
                ),
                TextBlock::new().text(format!("Count: {}", self.count)),
            ))
            .into()
    }
}
fn panic_text(work: impl FnOnce()) -> String {
    let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .expect_err("ambiguous mark must panic");
    error
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| error.downcast_ref::<&str>().unwrap().to_string())
}
#[test]
fn identical_variant_names_select_the_correct_control() {
    let harness = Harness::new(0);
    let mut page =
        Mounted::<Route>::from_window(&harness, Window::of::<Fixture>().at(Route::Home)).unwrap();
    let a = page.find(sidebar::Marks::Memory).unwrap();
    let b = page.find(performance::Marks::Memory).unwrap();
    assert_ne!(a, b);
    page.click(performance::Marks::Memory).settle();
    assert!(page.find_text("Count: 1").is_some());
    assert!(page.find(Repeated::Missing).is_none());
}
#[test]
fn duplicate_marks_require_a_subtree_and_report_paths() {
    let harness = Harness::new(0);
    let mut page = Mounted::<Fixture>::mount(harness.segment(), ()).unwrap();
    insta::assert_snapshot!(
        "mounted_ambiguity",
        panic_text(|| {
            let _ = page.find(Repeated::Run);
        })
    );
    insta::assert_snapshot!(
        "click_ambiguity",
        panic_text(|| {
            let _ = page.click(Repeated::Run);
        })
    );
    insta::assert_snapshot!(
        "tree_ambiguity",
        panic_text(|| {
            let _ = page.tree().find(Repeated::Run);
        })
    );
    page.within(Containers::Left).click(Repeated::Run).settle();
    page.within(Containers::Right).click(Repeated::Run).settle();
    assert!(page.find_text("Count: 2").is_some());
    assert!(
        page.tree()
            .find(Containers::Left)
            .unwrap()
            .find(Repeated::Run)
            .is_some()
    );
}
