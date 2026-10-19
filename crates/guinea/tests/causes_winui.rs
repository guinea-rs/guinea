#![cfg(all(windows, feature = "winui"))]

//! What a cause redrew, on a backend that draws a drain later: the renders it
//! set off are in its chain, and the state a layout or a part owns can be
//! read and driven from the mounted tree.

use guinea::app::Harness;
use guinea::prelude::*;
use guinea::observability::trace::Point;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Navigator, Page, PageCx, Routable, layout, page};
use windows_reactor::{Button, StackPanel, TextBlock, View};

#[guinea::slot]
pub struct Footer;

#[derive(Clone, Debug, Event)]
pub struct Sampled(pub u32);

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Load {
    pub level: u32,
}

#[reducer]
fn load(this: &mut Load, level: u32) {
    this.level = level;
}

#[derive(Clone, Debug)]
pub struct Reset;

#[derive(Debug)]
pub struct Meter {
    pub push: Push<Load>,
}

actor! {
    Meter {
        handlers { Reset }
        subscribes { Sampled }
    }
}

#[handler]
fn sampled(In(this, Sampled(level), _): In<Meter, Sampled>) {
    this.push.send(level);
}

#[handler]
fn reset(In(this, _, _): In<Meter, Reset>) {
    this.push.send(0);
}

feature! {
    pub Metering {
        exports { Load }
    }
}

#[installs]
fn metering(cx: &Install) -> anyhow::Result<Metering> {
    let (load, _) = cx.state::<Load>().driven_by(|push| Meter { push });
    Ok(Metering(load))
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Link {
    pub up: bool,
}

#[reducer]
fn link(this: &mut Link, up: bool) {
    this.up = up;
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
fn connect(In(this, _, _): In<Linker, Connect>) {
    this.push.send(true);
}

feature! {
    pub Linking {
        exports { Link }
    }
}

#[installs]
fn linking(cx: &Install) -> anyhow::Result<Linking> {
    let (link, _linker) = cx.state::<Link>().driven_by(|push| Linker { push });
    Ok(Linking(link))
}

#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Place {
    Processes,
    Services,
    Quiet,
    Watched,
}

/// Reads what it owns, and shows a part with state of its own in its footer.
#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Metering>(&())?;
        Ok(Self::default())
    }

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let (load, _) = cx.read::<Load>();
        let nav = cx.navigate::<Place>();

        let page = cx.outlet(&nav, |place, cx| match place {
            Place::Processes => cx.mount::<Processes>(&()),
            Place::Services => cx.mount::<Services>(&()),
            Place::Quiet => cx.mount::<Quiet>(&()),
            Place::Watched => cx.mount::<Watched>(&()),
        });
        let status = cx.mount::<Status>(&());
        let footer = cx.slot_or::<Footer>(status);

        StackPanel::new()
            .children((TextBlock::new().text(format!("load {}", load.level)), page, footer))
            .into()
    }
}

/// A part with state of its own.
#[derive(Default)]
pub struct Status;

#[page]
impl Page for Status {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Linking>(&())?;
        Ok(Self::default())
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
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("processes").into()
    }
}

thread_local! {
    static ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static NAVIGATOR: std::cell::RefCell<Option<Navigator<Place>>> =
        const { std::cell::RefCell::new(None) };
}

/// Hands the test the way it navigates, for a navigation that does not come
/// from a draw - only when the test asked, since the navigator holds the
/// window's place for as long as it is kept.
#[derive(Default)]
pub struct Services;

#[page]
impl Page for Services {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        if ASKED.get() {
            let nav = cx.navigate::<Place>();
            NAVIGATOR.with(|kept| *kept.borrow_mut() = Some(nav));
        }
        TextBlock::new().text("services").into()
    }
}

/// Told something it already shows, and sends nothing for it.
#[derive(Default)]
pub struct Quiet;

#[page]
impl Page for Quiet {
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

#[reducer]
fn rate(this: &mut Rate, ticks: u32) {
    this.ticks = ticks;
}

#[derive(Debug)]
pub struct Ticker {
    pub push: Push<Rate>,
}

actor! {
    Ticker {
        subscribes { Ticked }
    }
}

#[handler]
fn ticked(In(this, Ticked(ticks), _): In<Ticker, Ticked>) {
    this.push.send(ticks);
}

feature! {
    pub Ticking {
        exports { Rate }
    }
}

#[installs]
fn ticking(cx: &Install) -> anyhow::Result<Ticking> {
    let (rate, _) = cx.state::<Rate>().driven_by(|push| Ticker { push });
    Ok(Ticking(rate))
}

/// Owns what it reads, so leaving it tears that down.
#[derive(Default)]
pub struct Watched;

#[page]
impl Page for Watched {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Ticking>(&())?;
        Ok(Self::default())
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (rate, _) = cx.read::<Rate>();
        TextBlock::new().text(format!("ticks {}", rate.ticks)).into()
    }
}

fn rendered(segment: &str) -> impl Fn(&Point) -> bool + '_ {
    move |point| {
        matches!(point, Point::Render { segment: drawn, .. } if drawn.rsplit("::").next() == Some(segment))
    }
}

#[guinea::test(iterations = 4)]
fn a_published_event_has_the_layout_it_redrew_in_its_chain(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
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
fn a_layout_redrawn_does_not_redraw_the_page_under_it(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
    app.settle();

    let act = h.publish(Sampled(3));
    act.settle();
    app.settle();

    assert!(app.find_text("load 3").is_some(), "{:#?}", app.tree());
    assert!(
        !act.chain().has(rendered("Services")),
        "{:#?}",
        act.chain().points()
    );
}

#[guinea::test(iterations = 4)]
fn a_navigation_has_the_page_it_drew_in_its_chain(h: &mut Harness) {
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
    app.settle();

    let act = app.navigate(Place::Processes);
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
    ASKED.set(true);
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
    app.settle();
    ASKED.set(false);
    let nav = NAVIGATOR.with(|kept| kept.borrow_mut().take()).expect("services drew");

    let act = h.record("go", || {
        nav.to(Place::Processes);
    });
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
    let mut app = Mounted::routed::<Shell>(h, Place::Quiet).unwrap();
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
    let mut app = Mounted::routed::<Shell>(h, Place::Watched).unwrap();
    app.settle();
    h.publish(Ticked(1)).settle();

    let left = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.navigate(Place::Processes);
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
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
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
    let mut app = Mounted::routed::<Shell>(h, Place::Services).unwrap();
    app.settle();

    assert!(!app.state::<Link>().up);

    let act = app.act::<Link>(Connect);
    act.settle();
    app.settle();

    assert!(app.state::<Link>().up);
    assert!(app.find_text("link up true").is_some(), "{:#?}", app.tree());
    assert!(act.chain().has(rendered("Status")), "{:#?}", act.chain().points());
}
