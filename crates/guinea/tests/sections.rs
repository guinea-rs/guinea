//! What a feature sets up is that feature's.
//!
//! A page holds several features in one scope. Each claims its own state,
//! answers its own actions, hears events, runs actors and timers - and
//! devtools say whose each of these is. That holds for two features side
//! by side, and for a feature that installs another on its way: what it
//! sets up after the other one returned is its own again.

use std::time::Duration;

use guinea::app::Harness;
use guinea::core::actor::shape::name;
use guinea::core::timers;
use guinea::prelude::*;

/// Both features answer it, each for its own state.
#[derive(Clone, Debug)]
pub struct Refresh;

/// Both features hear it.
#[derive(Clone, Debug, Event)]
pub struct Tick;

mod clock {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Time(pub u32);

    #[reducer]
    fn time(this: &mut Time, by: u32) {
        this.0 += by;
    }

    pub struct Wind;

    #[derive(Debug)]
    pub struct Hand;

    actor! {
        Hand {
            handlers { Wind }
        }
    }

    #[handler]
    fn wind(_this: &mut Hand, _: Wind) {}

    feature! {
        pub Clock {
            exports { Time }
        }
    }

    #[installs]
    fn clock(cx: &FeatureInitContext) -> anyhow::Result<Clock> {
        let time = cx.state::<Time>().plain();

        cx.answers::<Refresh>(move |_| time.push(1));
        cx.subscribe(|_: Tick| {});
        cx.spawn(Hand);
        cx.repeat(Duration::from_secs(60), || {});

        Ok(Clock(time))
    }
}

mod weather {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Forecast(pub u32);

    #[reducer]
    fn forecast(this: &mut Forecast, by: u32) {
        this.0 += by;
    }

    pub struct Drift;

    #[derive(Debug)]
    pub struct Cloud;

    actor! {
        Cloud {
            handlers { Drift }
        }
    }

    #[handler]
    fn drift(_this: &mut Cloud, _: Drift) {}

    feature! {
        pub Weather {
            exports { Forecast }
        }
    }

    #[installs]
    fn weather(cx: &FeatureInitContext) -> anyhow::Result<Weather> {
        let forecast = cx.state::<Forecast>().plain();

        cx.answers::<Refresh>(move |_| forecast.push(1));
        cx.subscribe(|_: Tick| {});
        cx.spawn(Cloud);
        cx.repeat(Duration::from_secs(600), || {});

        Ok(Weather(forecast))
    }
}

/// A feature that installs the clock first, then sets up its own.
mod dashboard {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Pinned(pub u32);

    #[reducer]
    fn pinned(this: &mut Pinned, by: u32) {
        this.0 += by;
    }

    pub struct Unpin;

    #[derive(Debug)]
    pub struct Pin;

    actor! {
        Pin {
            handlers { Unpin }
        }
    }

    #[handler]
    fn unpin(_this: &mut Pin, _: Unpin) {}

    feature! {
        pub Dashboard {
            exports { Pinned }
        }
    }

    #[installs]
    fn dashboard(cx: &FeatureInitContext) -> anyhow::Result<Dashboard> {
        cx.install::<clock::Clock>(&())?;

        let pinned = cx.state::<Pinned>().plain();

        cx.answers::<Refresh>(move |_| pinned.push(1));
        cx.subscribe(|_: Tick| {});
        cx.spawn(Pin);
        cx.repeat(Duration::from_secs(5), || {});

        Ok(Dashboard(pinned))
    }
}

type Whose = (&'static str, &'static str, Option<&'static str>);

/// What the page runs and whose devtools say it is, as one table: kind,
/// what, feature.
fn whose(h: &Harness) -> Vec<Whose> {
    let scope = h.segment().context().scope;
    let short = |name: &'static str| name.rsplit("::").next().unwrap_or(name);
    let mut held = Vec::new();

    for listener in scope.listeners() {
        held.push(("listens", short(listener.event), listener.feature));
    }
    for actor in scope.actors_here() {
        held.push(("actor", short(actor.type_name), actor.owner.feature));
    }
    for timer in timers::running().into_iter().filter(|timer| timer.scope == Some(scope)) {
        held.push(("timer", "repeat", timer.feature));
    }

    held.sort();
    held
}

#[guinea::test(iterations = 1)]
fn two_features_in_one_page_each_answer_an_action_for_their_own_state(h: &mut Harness) {
    h.install::<clock::Clock>(&()).unwrap();
    h.install::<weather::Weather>(&()).unwrap();

    h.act::<weather::Forecast>(Refresh).settle();
    assert_eq!((h.state::<clock::Time>().0, h.state::<weather::Forecast>().0), (0, 1));

    h.act::<clock::Time>(Refresh).settle();
    assert_eq!((h.state::<clock::Time>().0, h.state::<weather::Forecast>().0), (1, 1));
}

#[guinea::test(iterations = 1)]
fn what_two_features_in_one_page_set_up_is_listed_as_each_one_s(h: &mut Harness) {
    h.install::<clock::Clock>(&()).unwrap();
    h.install::<weather::Weather>(&()).unwrap();

    let clock = Some(name::<clock::Clock>());
    let weather = Some(name::<weather::Weather>());
    assert_eq!(
        whose(h),
        [
            ("actor", "Cloud", weather),
            ("actor", "Hand", clock),
            ("listens", "Tick", clock),
            ("listens", "Tick", weather),
            ("timer", "repeat", clock),
            ("timer", "repeat", weather),
        ]
    );
}

#[guinea::test(iterations = 1)]
fn a_feature_that_installs_another_sets_up_the_rest_as_its_own(h: &mut Harness) {
    h.install::<dashboard::Dashboard>(&()).unwrap();

    let clock = Some(name::<clock::Clock>());
    let dashboard = Some(name::<dashboard::Dashboard>());
    assert_eq!(
        whose(h),
        [
            ("actor", "Hand", clock),
            ("actor", "Pin", dashboard),
            ("listens", "Tick", clock),
            ("listens", "Tick", dashboard),
            ("timer", "repeat", clock),
            ("timer", "repeat", dashboard),
        ]
    );
}

#[guinea::test(iterations = 1)]
fn a_feature_that_installs_another_answers_for_its_own_state_and_the_other_for_its(
    h: &mut Harness,
) {
    h.install::<dashboard::Dashboard>(&()).unwrap();

    h.act::<dashboard::Pinned>(Refresh).settle();
    assert_eq!((h.state::<clock::Time>().0, h.state::<dashboard::Pinned>().0), (0, 1));

    h.act::<clock::Time>(Refresh).settle();
    assert_eq!((h.state::<clock::Time>().0, h.state::<dashboard::Pinned>().0), (1, 1));
}
