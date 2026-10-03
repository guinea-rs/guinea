#![cfg(all(windows, feature = "winui"))]

//! The README's example, compiled and clicked. `cargo xtask docs` copies what
//! is between the marks into the README.

//@show counter
use guinea::prelude::*;
use guinea::winui::{Page, PageCx, Window, page, run};
use guinea::winui::reactor::{Button, ChildrenControl, ContentControl, StackPanel, TextBlock, View};

/// The state.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

/// How it changes.
#[reducer]
fn count(this: &mut Count, by: u32) {
    this.0 += by;
}

/// What the UI asks for.
pub struct Add(pub u32);

/// The domain: answers `Add`, and pushes the new state.
#[derive(Debug)]
pub struct Counting {
    push: Push<Count>,
}

actor! {
    Counting {
        handlers { Add }
    }
}

#[handler]
fn add(this: &mut Counting, Add(by): Add) {
    this.push.send(by);
}

// What the feature publishes to the pages that install it, or sit below.
feature! {
    pub Counter {
        exports { Count }
    }
}

#[installs]
fn counter(cx: &FeatureInitContext) -> anyhow::Result<Counter> {
    let (count, _) = cx.state::<Count>().driven_by(|push| Counting { push });
    Ok(Counter(count))
}

#[derive(Default)]
pub struct Home;

#[page]
impl Page for Home {
    type Installs = Counter;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Counter> {
        ctx.install(&())
    }

    fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
        let (count, dispatch) = cx.read::<Count, _>();

        StackPanel::new()
            .children((
                TextBlock::new().text(count.0.to_string()),
                Button::new()
                    .on_click(move || dispatch.emit(Add(1)))
                    .content(TextBlock::new().text("Add")),
            ))
            .into()
    }
}

routes! {
    Route {
        page(Home) { }
    }
}

fn main() -> anyhow::Result<()> {
    run(GuineaApp::new(), Window::new().title("Counter"), |_| {
        Route::Home {}
    })
}
//@show-end

#[cfg(feature = "harness")]
#[guinea::test(iterations = 4)]
fn a_click_on_add_counts(h: &mut guinea::app::Harness) {
    let mut page = guinea::winui::harness::Mounted::<Home>::mount(&h.segment(), ()).unwrap();

    page.click_text("Add").settle();
    page.settle();

    assert!(page.find_text("1").is_some(), "{:#?}", page.tree());
}
