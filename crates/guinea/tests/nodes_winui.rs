#![cfg(all(windows, feature = "winui"))]

//! A page's node as its view draws it and its messages reach it: what it
//! reads, and which node a message it sent ends up at.

use std::panic::{AssertUnwindSafe, catch_unwind};

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;
use guinea::winui::{Page, PageCx, page};
use windows_reactor::{TextBlock, View};

fn said(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string()))
        .unwrap_or_default()
}

mod twice {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Level(pub u32);

    #[reducer]
    fn level(this: &mut Level, to: u32) {
        this.0 = to;
    }

    #[derive(Default)]
    pub struct TwiceReader;

    #[page]
    impl Page for TwiceReader {
        fn install(ctx: &Install) -> anyhow::Result<Self> {
            ctx.state::<Level>().seed(Level(3)).plain();
            Ok(Self)
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (first, _) = cx.read::<Level>();
            let (second, _) = cx.read::<Level>();
            TextBlock::new().text(format!("level {} {}", first.0, second.0)).into()
        }
    }

    #[guinea::test(iterations = 1)]
    fn a_view_may_read_the_same_reducer_twice(h: &mut Harness) {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            Mounted::<TwiceReader>::mount(h.segment(), ())
                .map(|page| page.find_text("level 3 3").is_some())
                .map_err(|error| format!("{error:#}"))
        }));

        let outcome = outcome.map_err(|panic| said(&*panic));
        assert_eq!(outcome, Ok(Ok(true)));
    }
}
