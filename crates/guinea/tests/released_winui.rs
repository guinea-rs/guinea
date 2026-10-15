#![cfg(all(windows, feature = "winui"))]

//! What a page's actors hold goes when the page does, however many times the
//! window comes back to it.

use std::cell::Cell;

use guinea::app::Harness;
use guinea::core::actor::event_bus::Event;
use guinea::prelude::*;
use guinea::winui::harness::Mounted;
use guinea::winui::{Layout, LayoutCx, Page, PageCx, Routable, UpdateCx, layout, page};
use windows_reactor::{TextBlock, View};

thread_local! {
    static ALIVE: Cell<usize> = const { Cell::new(0) };
}

fn alive() -> usize {
    ALIVE.with(Cell::get)
}

#[derive(Debug)]
pub struct Alive;

impl Alive {
    fn new() -> Self {
        ALIVE.with(|alive| alive.set(alive.get() + 1));
        Self
    }
}

impl Drop for Alive {
    fn drop(&mut self) {
        ALIVE.with(|alive| alive.set(alive.get() - 1));
    }
}

#[derive(Clone, Debug)]
pub struct Ping;

impl Event for Ping {}

#[derive(Clone, Debug)]
pub struct Pinged;

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Pings(pub u32);

#[reducer]
fn pings(this: &mut Pings, _pinged: Pinged) {
    this.0 += 1;
}

#[derive(Debug)]
pub struct Listener {
    push: Push<Pings>,
    _alive: Alive,
}

actor! {
    Listener {
        subscribes { Ping }
    }
}

#[handler]
fn ping(this: &mut Listener, _: Ping) {
    this.push.send(Pinged);
}

#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Place {
    Listening,
    Menu,
    Elsewhere,
    Reading,
    Quiet,
}

#[derive(Default)]
pub struct Frame;

#[layout]
impl Layout for Frame {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Place>();
        cx.outlet(&nav, |place, cx| match place {
            Place::Listening => cx.mount::<Listening>(&()),
            Place::Menu => cx.mount::<Menu>(&()),
            Place::Elsewhere => cx.mount::<Elsewhere>(&()),
            Place::Reading => cx.mount::<PingArea>(&()),
            Place::Quiet => cx.mount::<QuietArea>(&()),
        })
    }
}

#[derive(Default)]
pub struct Listening;

#[page]
impl Page for Listening {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.state::<Pings>().driven_by(|push| Listener { push, _alive: Alive::new() });
        Ok(Self::default())
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (pings, _) = cx.read::<Pings>();
        TextBlock::new().text(format!("pings {}", pings.0)).into()
    }
}

#[derive(Clone, Debug)]
pub struct PressedAway;

impl Event for PressedAway {}

#[derive(Debug)]
pub struct Dismiss;

#[derive(Default)]
pub struct Menu;

#[page]
impl Page for Menu {
    type Message = Dismiss;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Pinging>(&())?;
        Ok(Self::default())
    }

    fn update(&mut self, _message: Dismiss, cx: &mut UpdateCx<'_, Self>) {
        let (_pings, _) = cx.read::<Pings>();
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let away = cx.on(|message: Dismiss| message);
        let bus = cx.bus();
        cx.use_effect_guard("released::menu_closes_on_press_away", (), move || {
            bus.subscribe_fn(move |_: PressedAway| away.call(Dismiss))
        });
        TextBlock::new().text("menu").into()
    }
}

#[derive(Default)]
pub struct Elsewhere;

#[page]
impl Page for Elsewhere {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("elsewhere").into()
    }
}

feature! {
    pub Pinging {
        exports { Pings }
    }
}

#[installs]
fn pinging(cx: &FeatureInitContext) -> anyhow::Result<Pinging> {
    let (pings, _) = cx.state::<Pings>().driven_by(|push| Listener { push, _alive: Alive::new() });
    Ok(Pinging(pings))
}

#[derive(Default)]
pub struct PingArea;

#[layout]
impl Layout for PingArea {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Pinging>(&())?;
        Ok(Self::default())
    }

    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        cx.mount::<Reading>(&()).into()
    }
}

#[derive(Default)]
pub struct QuietArea;

#[layout]
impl Layout for QuietArea {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        cx.mount::<Quiet>(&()).into()
    }
}

#[derive(Default)]
pub struct Reading;

#[page]
impl Page for Reading {
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (pings, _) = cx.read::<Pings>();
        TextBlock::new().text(format!("pings {}", pings.0)).into()
    }
}

#[derive(Default)]
pub struct Quiet;

#[page]
impl Page for Quiet {
    fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("quiet").into()
    }
}

#[guinea::test(iterations = 4)]
fn a_message_for_a_page_that_was_left_meanwhile_is_dropped(h: &mut Harness) {
    let mut app = Mounted::routed::<Frame>(h, Place::Menu).unwrap();

    let navigated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _pressed = h.publish(PressedAway);
        app.navigate(Place::Elsewhere);
    }));

    assert!(navigated.is_ok(), "the left page's update ran on a removed scope");
    assert!(app.find_text("elsewhere").is_some());
}

#[guinea::test(iterations = 1)]
fn a_layout_s_feature_goes_with_the_area_every_time(h: &mut Harness) {
    let mut app = Mounted::routed::<Frame>(h, Place::Reading).unwrap();

    for visit in 1..=3 {
        h.publish(Ping).settle();
        app.settle();
        assert!(app.find_text("pings 1").is_some(), "visit {visit}: the page heard its ping");
        assert_eq!(alive(), 1, "visit {visit}: one actor while the area is open");
        assert_eq!(h.bus().count_subscribers::<Ping>(), 1, "visit {visit}: one subscription");

        app.navigate(Place::Quiet);
        assert_eq!(alive(), 0, "visit {visit}: the actor outlived its area");
        assert_eq!(h.bus().count_subscribers::<Ping>(), 0, "visit {visit}: the subscription outlived its area");

        app.navigate(Place::Reading);
    }
}

#[guinea::test(iterations = 1)]
fn a_page_s_actor_and_its_subscription_go_with_the_page_every_time(h: &mut Harness) {
    let mut app = Mounted::routed::<Frame>(h, Place::Listening).unwrap();

    for visit in 1..=3 {
        h.publish(Ping).settle();
        app.settle();
        assert!(app.find_text("pings 1").is_some(), "visit {visit}: the page heard its ping");
        assert_eq!(alive(), 1, "visit {visit}: one actor while the page is open");
        assert_eq!(h.bus().count_subscribers::<Ping>(), 1, "visit {visit}: one subscription");

        app.navigate(Place::Elsewhere);
        assert_eq!(alive(), 0, "visit {visit}: the actor outlived its page");
        assert_eq!(h.bus().count_subscribers::<Ping>(), 0, "visit {visit}: the subscription outlived its page");

        app.navigate(Place::Listening);
    }
}
