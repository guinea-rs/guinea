#![cfg(feature = "eframe")]
#![allow(dead_code)]

//! The egui examples on the book's page about pages and layouts, compiled.
//! `cargo xtask docs` copies what is between the marks into it.

use guinea::prelude::*;

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

//@show egui: a page
use guinea::eframe::{Page, PageCx, page};

pub struct CounterPage {
    step: u32,
}

#[page]
impl Page for CounterPage {
    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Counter>(&())?;
        Ok(Self { step: 1 })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (count, dispatch) = cx.read::<Count>();
        let ui = cx.ui();

        ui.label(format!("{} by {}", count.0, self.step));
        if ui.button("Add").clicked() {
            dispatch.emit(Add(self.step));
        }
        if ui.button("Bigger").clicked() {
            self.step += 1;
        }
    }
}
//@show-end

mod shell {
    use super::*;
    use guinea::eframe::{Layout, LayoutCx, Routable, layout};

    #[derive(Default)]
    pub struct About;

    #[page]
    impl Page for About {
        fn render(&mut self, cx: &mut PageCx<'_, Self>) {
            cx.ui().label("guinea");
        }
    }

    //@show egui: a layout
    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum Place {
        Counter,
        About,
    }

    #[derive(Default)]
    pub struct Shell;

    #[layout]
    impl Layout for Shell {
        fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
            let nav = cx.navigate::<Place>();
            if cx.ui().button("About").clicked() {
                let _ = nav.to(Place::About);
            }

            let page = cx.outlet(&nav, |place, cx| match place {
                Place::Counter => cx.mount::<CounterPage>(&()),
                Place::About => cx.mount::<About>(&()),
            });
            page.draw(cx.ui());
        }
    }
    //@show-end

    //@show egui: an application
    use guinea::eframe::{Window, run};

    fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
        Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
    }

    fn main() -> anyhow::Result<()> {
        run(app)
    }
    //@show-end
}
