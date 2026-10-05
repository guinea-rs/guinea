#![cfg(all(windows, feature = "winui"))]

//! What a cause redrew, on a backend that draws a drain later: the renders it
//! set off are in its chain, and the state a layout or a part owns can be
//! read and driven from the mounted tree.

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::observability::trace::Point;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, layout, page};
use windows_reactor::{Button, StackPanel, TextBlock, View};

#[guinea::slot]
pub struct Footer;

#[derive(Clone, Debug, Event)]
pub struct Sampled(pub u32);

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Load {
    pub level: u32,
}

impl Reducer for Load {
    type Update = u32;

    fn reduce(&mut self, level: u32) {
        self.level = level;
    }
}

#[derive(Clone, Debug)]
pub struct Reset;

#[derive(Debug)]
pub struct Meter {
    pub push: Push<Load>,
}

actor! {
    Meter {
        handlers { Sampled, Reset }
    }
}

#[handler]
fn sampled(this: &mut Meter, Sampled(level): Sampled) {
    this.push.send(level);
}

#[handler]
fn reset(this: &mut Meter, _reset: Reset) {
    this.push.send(0);
}

feature! {
    pub Metering {
        exports { Load }
    }
}

#[installs]
fn metering(cx: &FeatureInitContext) -> anyhow::Result<Metering> {
    let (load, meter) = cx.state::<Load>().driven_by(|push| Meter { push });
    meter.subscribe_on::<Sampled>(Bus::Global);
    Ok(Metering(load))
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Link {
    pub up: bool,
}

impl Reducer for Link {
    type Update = bool;

    fn reduce(&mut self, up: bool) {
        self.up = up;
    }
}

#[derive(Clone, Debug)]
pub struct Connect;

#[derive(Debug)]
pub struct Linker {
    pub push: Push<Link>,
}

actor! {
    Linker {
        handlers { Connect }
    }
}

#[handler]
fn connect(this: &mut Linker, _connect: Connect) {
    this.push.send(true);
}

feature! {
    pub Linking {
        exports { Link }
    }
}

#[installs]
fn linking(cx: &FeatureInitContext) -> anyhow::Result<Linking> {
    let (link, _linker) = cx.state::<Link>().driven_by(|push| Linker { push });
    Ok(Linking(link))
}

/// Reads what it owns, and shows a part in its footer.
#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    type Params = ShellParams;
    type Installs = Metering;

    fn install(ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<Metering> {
        ctx.install(&())
    }

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let (load, _) = cx.read::<Load>();

        StackPanel::new()
            .children((
                TextBlock::new().text(format!("load {}", load.level)),
                cx.outlet(),
                cx.slot::<Footer>(),
            ))
            .into()
    }
}

/// A part with state of its own.
#[derive(Default)]
pub struct Status;

#[page]
impl Page for Status {
    type Installs = Linking;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Linking> {
        ctx.install(&())
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (link, _) = cx.read::<Link>();
        TextBlock::new().text(format!("link up {}", link.up)).into()
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

thread_local! {
    static NAVIGATOR: std::cell::RefCell<Option<guinea::router::NavigateHandle<guinea::winui::WinUi, Route>>> =
        const { std::cell::RefCell::new(None) };
}

/// Hands the test the way it navigates, for a navigation that does not come
/// from a draw.
#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    type Params = ServicesParams;

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Route>();
        NAVIGATOR.with(|kept| *kept.borrow_mut() = Some(nav));
        TextBlock::new().text("services").into()
    }
}

/// Told something it already shows, and sends nothing for it.
#[derive(Default)]
pub struct Quiet;

#[page]
impl Page for Quiet {
    type Params = QuietParams;
    type Message = ();

    fn update(&mut self, _message: (), _cx: &mut guinea::winui::UpdateCx<'_, Self>) {}

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        Button::new()
            .on_click(cx.on_some(|()| None))
            .content(TextBlock::new().text("same again"))
            .into()
    }
}

#[derive(Clone, Debug, Event)]
pub struct Ticked(pub u32);

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Rate {
    pub ticks: u32,
}

impl Reducer for Rate {
    type Update = u32;

    fn reduce(&mut self, ticks: u32) {
        self.ticks = ticks;
    }
}

#[derive(Debug)]
pub struct Ticker {
    pub push: Push<Rate>,
}

actor! {
    Ticker {
        handlers { Ticked }
    }
}

#[handler]
fn ticked(this: &mut Ticker, Ticked(ticks): Ticked) {
    this.push.send(ticks);
}

feature! {
    pub Ticking {
        exports { Rate }
    }
}

#[installs]
fn ticking(cx: &FeatureInitContext) -> anyhow::Result<Ticking> {
    let (rate, ticker) = cx.state::<Rate>().driven_by(|push| Ticker { push });
    ticker.subscribe_on::<Ticked>(Bus::Global);
    Ok(Ticking(rate))
}

/// Owns what it reads, so leaving it tears that down.
#[derive(Default)]
pub struct Watched;

#[page]
impl Page for Watched {
    type Params = WatchedParams;
    type Installs = Ticking;

    fn install(ctx: &FeatureInitContext, _params: &WatchedParams) -> anyhow::Result<Ticking> {
        ctx.install(&())
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (rate, _) = cx.read::<Rate>();
        TextBlock::new().text(format!("ticks {}", rate.ticks)).into()
    }
}

guinea::routes! {
    Route {
        layout(Shell) {
            part(Status) => Footer
            page(Processes) { }
            page(Services) { }
            page(Quiet) { }
            page(Watched) { }
        }
    }
}

fn rendered(segment: &str) -> impl Fn(&Point) -> bool + '_ {
    move |point| {
        matches!(point, Point::Render { segment: drawn, .. } if drawn.rsplit("::").next() == Some(segment))
    }
}

#[guinea::test(iterations = 4)]
fn a_published_event_has_the_layout_it_redrew_in_its_chain(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Services {}).unwrap();
    app.settle();

    let act = h.publish(Sampled(3));
    act.settle();
    app.settle();

    assert!(app.find_text("load 3").is_some(), "{:#?}", app.tree());
    assert!(
        act.chain().has(rendered("Shell")),
        "{:#?}",
        act.chain().points()
    );
}

#[guinea::test(iterations = 4)]
fn a_navigation_has_the_page_it_drew_in_its_chain(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Services {}).unwrap();
    app.settle();

    let act = app.navigate(Route::Processes {});
    app.settle();

    assert!(app.find_text("processes").is_some(), "{:#?}", app.tree());
    assert!(
        act.chain().has(rendered("Processes")),
        "{:#?}",
        act.chain().points()
    );
}

#[guinea::test(iterations = 4)]
fn a_navigation_drawn_a_drain_later_has_what_it_drew_in_its_chain(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Services {}).unwrap();
    app.settle();
    let nav = NAVIGATOR.with(|kept| kept.borrow_mut().take()).expect("services drew");

    let act = h.record("go", || nav.to(Route::Processes {}));
    app.settle();

    assert!(app.find_text("processes").is_some(), "{:#?}", app.tree());
    assert!(
        act.chain().has(rendered("Processes")),
        "{:#?}",
        act.chain().points()
    );
}

#[guinea::test(iterations = 4)]
fn a_callback_that_sends_nothing_redraws_nothing(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Quiet {}).unwrap();
    app.settle();

    let act = app.click_text("same again");
    app.settle();

    assert!(
        !act.chain().has(rendered("Quiet")),
        "{:#?}",
        act.chain().points()
    );
}

#[guinea::test(iterations = 4)]
fn a_page_left_with_a_redraw_pending_is_not_drawn_again(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Watched {}).unwrap();
    app.settle();
    h.publish(Ticked(1)).settle();

    let left = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.navigate(Route::Processes {});
        app.settle();
        app.find_text("processes").is_some()
    }));

    let said = left.as_ref().map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string()))
            .unwrap_or_default()
    });
    assert_eq!(said, Ok(&true));
}

#[guinea::test(iterations = 4)]
fn state_a_layout_owns_is_read_and_driven_from_the_mounted_tree(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Services {}).unwrap();
    app.settle();
    h.publish(Sampled(5)).settle();
    app.settle();

    assert_eq!(app.state::<Load>().level, 5);

    let act = app.act::<Load>(Reset);
    act.settle();
    app.settle();

    assert_eq!(app.state::<Load>().level, 0);
    assert!(act.chain().has(rendered("Shell")), "{:#?}", act.chain().points());
}

#[guinea::test(iterations = 4)]
fn state_a_part_owns_is_read_and_driven_from_the_mounted_tree(h: &mut Harness) {
    let mut app = Mounted::routed(h, Route::Services {}).unwrap();
    app.settle();

    assert!(!app.state::<Link>().up);

    let act = app.act::<Link>(Connect);
    act.settle();
    app.settle();

    assert!(app.state::<Link>().up);
    assert!(app.find_text("link up true").is_some(), "{:#?}", app.tree());
    assert!(act.chain().has(rendered("Status")), "{:#?}", act.chain().points());
}
