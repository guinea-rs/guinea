//! What the tool watching an application sends into it: an action to the
//! window it names, an event onto the application's bus.

use guinea::app::roots::Root;
use guinea::app::{GuineaHost, Observed};
use guinea::prelude::*;

#[derive(Clone, Debug, serde::Deserialize, guinea::Remote)]
#[remote(action)]
pub struct Rename(pub String);

#[derive(Clone, Debug, serde::Deserialize, Event, guinea::Remote)]
#[remote(event)]
pub struct Renamed(pub String);

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Name(pub String);

#[reducer]
fn name(this: &mut Name, renamed: Renamed) {
    this.0 = renamed.0;
}

#[derive(Debug)]
pub struct Namer {
    pub push: Push<Name>,
}

actor! {
    Namer {
        handlers { Rename }
        publishes { Renamed }
        subscribes { Renamed }
    }
}

#[handler]
fn rename(In(_, Rename(name), cx): In<Namer, Rename>) {
    cx.publish(Renamed(name));
}

#[handler]
fn renamed(In(this, renamed, _): In<Namer, Renamed>) {
    this.push.send(renamed);
}

feature! {
    pub Naming {
        exports { Name }
    }
}

#[installs]
fn naming(cx: &FeatureInitContext) -> anyhow::Result<Naming> {
    let (name, _) = cx.state::<Name>().driven_by(|push| Namer { push });
    Ok(Naming(name))
}

plugin! {
    pub Watches {
        provides { Observed }
    }
}

#[builds]
fn watches(app: &mut PluginBuilder) -> anyhow::Result<Watches> {
    Ok(Watches(app.observed()))
}

/// An application with one window, a page in it naming things, and the
/// handle a tool watches it through.
struct Named {
    observed: Observed,
    page: guinea_core::scope::ScopeGuard,
    _window: Root,
    _host: GuineaHost,
}

fn named() -> Named {
    let (host, observed) = guinea::app::start("test", |app| {
        let Watches(observed) = app.plugin::<Watches>(())?;
        Ok(observed)
    })
    .expect("install");
    let window = Root::under(&host.context());
    let page = window
        .install(|cx| cx.install::<Naming>(&()).map(|_| ()))
        .expect("the page installs");

    Named {
        observed,
        page,
        _window: window,
        _host: host,
    }
}

fn name_read(named: &Named) -> Option<String> {
    named
        .observed
        .state(named.page.scope().id(), std::any::type_name::<Name>())
        .map(|state| state.state.preview)
}

#[test]
fn an_action_sent_by_a_tool_reaches_the_page_in_the_newest_window_that_answers_it() {
    let named = named();

    let sent = named.observed.act(None, "Rename", r#""guinea""#);

    assert!(sent.is_ok(), "{sent:?}");
    assert!(name_read(&named).is_some_and(|name| name.contains("guinea")), "{:?}", name_read(&named));
}

#[test]
fn a_tool_is_told_what_actions_and_events_it_may_send() {
    let named = named();

    let offered = (named.observed.actions(), named.observed.events());

    assert!(
        offered.0.contains(&"Rename") && offered.1.contains(&"Renamed"),
        "{offered:?}"
    );
}

#[test]
fn an_action_for_a_window_that_is_not_open_is_refused() {
    let named = named();

    let refused = named.observed.act(Some(u64::MAX), "Rename", r#""guinea""#);

    assert!(refused.is_err_and(|refused| refused.contains("no window")));
}

#[test]
fn a_tool_watches_an_application_a_harness_drives_and_both_hear_what_it_did() {
    let mut h = guinea::app::Harness::new(1);
    let Watches(observed) = h.plugin::<Watches>(()).expect("the plugin builds");
    h.install::<Naming>(&()).expect("the feature installs");

    let watching = observed.watch(|| {});
    let act = h.act::<Name>(Renamed("guinea".into()));
    act.settle();
    let heard = watching.as_ref().map(|watching| {
        watching.take().trace.iter().any(|(_, trace)| {
            matches!(
                trace,
                guinea::observability::trace::Trace::Begin(record) if record.id == act.cause()
            )
        })
    });

    assert_eq!((heard.ok(), act.is_settled()), (Some(true), true));
}

#[test]
fn an_event_sent_by_a_tool_goes_out_on_the_application_s_bus() {
    let named = named();

    let sent = named.observed.publish("Renamed", r#""code""#);

    assert!(sent.is_ok(), "{sent:?}");
    assert!(name_read(&named).is_some_and(|name| name.contains("code")), "{:?}", name_read(&named));
}
