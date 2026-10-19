#![cfg(all(windows, feature = "winui"))]
#![allow(dead_code)]

//! The WinUI examples on the book's page about pages and layouts, run.
//! `cargo xtask docs` copies what is between the marks into it.

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;

mod counting {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Count(pub u32);

    #[reducer]
    fn count(this: &mut Count, by: u32) {
        this.0 += by;
    }

    pub struct Add(pub u32);

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
    fn add(In(this, Add(by), _): In<Counting, Add>) {
        this.push.send(by);
    }

    feature! {
        pub Counter {
            exports { Count }
        }
    }

    #[installs]
    fn counter(cx: &Install) -> anyhow::Result<Counter> {
        let (count, _) = cx.state::<Count>().driven_by(|push| Counting { push });
        Ok(Counter(count))
    }
}

use counting::{Add, Count, Counter};

//@show winui: a page
use guinea::winui::reactor::{Button, StackPanel, TextBlock, View};
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};

#[derive(guinea::Mark)]
pub enum Marks {
    Add,
    Bigger,
}

pub struct CounterPage {
    step: u32,
}

pub enum Msg {
    Bigger,
}

#[page]
impl Page for CounterPage {
    type Message = Msg;

    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Counter>(&())?;
        Ok(Self { step: 1 })
    }

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::Bigger => self.step += 1,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (count, dispatch) = cx.read::<Count>();
        let step = self.step;

        StackPanel::new()
            .children((
                TextBlock::new().text(format!("{} by {step}", count.0)),
                Button::new()
                    .content(TextBlock::new().text("Add"))
                    .on_click(move || dispatch.emit(Add(step)))
                    .mark(Marks::Add),
                Button::new()
                    .content(TextBlock::new().text("Bigger"))
                    .on_click(cx.on(|()| Msg::Bigger))
                    .mark(Marks::Bigger),
            ))
            .into()
    }
}
//@show-end

//@show winui: testing a page
#[guinea::test]
fn adds_the_step_it_was_told(h: &mut Harness) {
    let mut page = Mounted::<CounterPage>::mount(h.segment(), ()).unwrap();

    page.click(Marks::Bigger).settle();
    page.click(Marks::Add).settle();
    page.settle();

    assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
}
//@show-end

mod shell {
    use super::*;
    use guinea::winui::{Layout, LayoutCx, Routable, layout};

    #[derive(Default)]
    pub struct About;

    #[page]
    impl Page for About {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("guinea").into()
        }
    }

    //@show winui: a layout
    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum Place {
        Counter,
        About,
    }

    #[derive(Default)]
    pub struct Shell;

    #[layout]
    impl Layout for Shell {
        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<Place>();
            let to_about = nav.clone();

            StackPanel::new()
                .children((
                    Button::new()
                        .content(TextBlock::new().text("About"))
                        .on_click(move || {
                            let _ = to_about.to(Place::About);
                        }),
                    cx.outlet(&nav, |place, cx| match place {
                        Place::Counter => cx.mount::<CounterPage>(&()),
                        Place::About => cx.mount::<About>(&()),
                    }),
                ))
                .into()
        }
    }
    //@show-end

    //@show winui: testing a layout
    #[guinea::test]
    fn the_about_button_moves_the_window(h: &mut Harness) {
        let mut shell = Mounted::<Shell>::mount_at(h.segment(), (), Place::Counter).unwrap();
        assert!(shell.find_text("0 by 1").is_some(), "{:#?}", shell.tree());

        shell.click_text("About").settle();
        shell.settle();

        assert_eq!(shell.navigator::<Place>().current(), Some(Place::About));
        assert!(shell.find_text("guinea").is_some(), "{:#?}", shell.tree());
    }
    //@show-end

    //@show winui: an application
    use guinea::winui::{Window, run};

    fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
        Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
    }

    fn main() -> anyhow::Result<()> {
        run(app)
    }
    //@show-end
}
