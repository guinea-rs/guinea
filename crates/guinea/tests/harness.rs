//! The harness on the race it exists for: two searches in flight, and the
//! answer that lands last decides what the page shows.
//!
//! The user types `gu`, then `guinea`. A search takes as long as it takes, so
//! the one for `gu` can land after the one for `guinea` - and a searcher that
//! shows whatever arrived last shows results for a query nobody is looking at
//! any more. Under tokio that happens in production and almost never in a
//! test; here the seeds walk the orders until one does.

use guinea::app::Harness;
use guinea::prelude::*;

const CATALOGUE: [&str; 6] = ["guinea", "guinea-app", "gui", "gum", "gulp", "gust"];

#[derive(Default, Clone, PartialEq, Debug, serde::Serialize)]
pub struct Results {
    pub query: String,
    pub found: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub query: String,
    pub found: Vec<&'static str>,
}

#[reducer]
fn results(this: &mut Results, update: Found) {
    this.query = update.query;
    this.found = update.found;
}

#[derive(Clone, Debug)]
pub struct Query(pub String);

/// Takes as long as the seed says.
async fn search(text: String) -> Found {
    let found: Vec<&'static str> = CATALOGUE
        .iter()
        .copied()
        .filter(|name| name.contains(text.as_str()))
        .collect();

    guinea::core::executor::random_delay().await;

    Found { query: text, found }
}

/// Shows whatever answer arrives, in the order they arrive.
mod naive {
    use super::*;

    #[derive(Debug)]
    pub struct Searcher {
        pub push: Push<Results>,
    }

    actor! {
        Searcher {
            handlers { Query => { task Found }, Found }
        }
    }

    #[handler]
    fn query(_this: &mut Searcher, Query(text): Query, cx: Cx) {
        cx.task().spawn(search(text));
    }

    #[handler]
    fn found(this: &mut Searcher, found: Found) {
        this.push.send(found);
    }

    feature! {
        pub Search {
            exports { Results }
        }
    }

    #[installs]
    fn searching(cx: &FeatureInitContext) -> anyhow::Result<Search> {
        let (results, _) = cx.state::<Results>().driven_by(|push| Searcher { push });
        Ok(Search(results))
    }
}

/// Numbers every query and shows an answer only if it is for the latest.
mod latest {
    use super::*;

    #[derive(Debug)]
    pub struct Searcher {
        pub push: Push<Results>,
        pub asked: u64,
    }

    #[derive(Clone, Debug)]
    pub struct Answered {
        pub asked: u64,
        pub found: Found,
    }

    actor! {
        Searcher {
            handlers { Query => { task Answered }, Answered }
        }
    }

    #[handler]
    fn query(this: &mut Searcher, Query(text): Query, cx: Cx) {
        this.asked += 1;

        let asked = this.asked;
        cx.task().spawn(async move {
            Answered {
                asked,
                found: search(text).await,
            }
        });
    }

    #[handler]
    fn answered(this: &mut Searcher, answered: Answered) {
        if answered.asked == this.asked {
            this.push.send(answered.found);
        }
    }

    feature! {
        pub Search {
            exports { Results }
        }
    }

    #[installs]
    fn searching(cx: &FeatureInitContext) -> anyhow::Result<Search> {
        let (results, _) = cx.state::<Results>().driven_by(|push| Searcher { push, asked: 0 });
        Ok(Search(results))
    }
}

/// Leaves the numbering to `cx.task().latest()`: a new query replaces the
/// search in flight, and an answer that comes anyway is dropped.
mod by_policy {
    use super::*;

    #[derive(Debug)]
    pub struct Searcher {
        pub push: Push<Results>,
    }

    actor! {
        Searcher {
            handlers { Query => { task Found }, Found }
        }
    }

    #[handler]
    fn query(_this: &mut Searcher, Query(text): Query, cx: Cx) {
        cx.task().latest().spawn(search(text));
    }

    #[handler]
    fn found(this: &mut Searcher, found: Found) {
        this.push.send(found);
    }

    feature! {
        pub Search {
            exports { Results }
        }
    }

    #[installs]
    fn searching(cx: &FeatureInitContext) -> anyhow::Result<Search> {
        let (results, _) = cx.state::<Results>().driven_by(|push| Searcher { push });
        Ok(Search(results))
    }
}

fn type_gu_then_guinea(h: &Harness) {
    h.dispatch::<Results>().emit(Query("gu".into()));
    h.dispatch::<Results>().emit(Query("guinea".into()));
    h.settled();
}

#[guinea::test(iterations = 200)]
#[should_panic(expected = "SEED=")]
fn some_order_shows_an_answer_to_a_query_nobody_is_looking_at(h: &mut Harness) {
    h.install::<naive::Search>(&()).unwrap();
    type_gu_then_guinea(h);

    assert_eq!(h.state::<Results>().query, "guinea");
}

#[guinea::test(iterations = 200)]
fn in_every_order_the_latest_query_wins(h: &mut Harness) {
    h.install::<latest::Search>(&()).unwrap();
    type_gu_then_guinea(h);

    let results = h.state::<Results>();
    assert_eq!(results.query, "guinea");
    assert_eq!(results.found, ["guinea", "guinea-app"]);
    assert_eq!(h.stuck(), 0);
}

#[guinea::test(iterations = 200)]
fn in_every_order_the_latest_query_wins_when_the_policy_says_so(h: &mut Harness) {
    h.install::<by_policy::Search>(&()).unwrap();
    type_gu_then_guinea(h);

    let results = h.state::<Results>();
    assert_eq!(results.query, "guinea");
    assert_eq!(results.found, ["guinea", "guinea-app"]);
    assert_eq!(h.stuck(), 0);
}

/// Waits the way the example's metrics do: `tokio::time::sleep` inside the
/// background task.
mod sleepy {
    use super::*;

    #[derive(Debug)]
    pub struct Searcher {
        pub push: Push<Results>,
    }

    actor! {
        Searcher {
            handlers { Query => { task Found }, Found }
        }
    }

    #[handler]
    fn query(_this: &mut Searcher, Query(text): Query, cx: Cx) {
        cx.task().spawn(async move {
            guinea::core::__private::tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            Found { query: text, found: Vec::new() }
        });
    }

    #[handler]
    fn found(this: &mut Searcher, found: Found) {
        this.push.send(found);
    }

    feature! {
        pub Search {
            exports { Results }
        }
    }

    #[installs]
    fn searching(cx: &FeatureInitContext) -> anyhow::Result<Search> {
        let (results, _) = cx.state::<Results>().driven_by(|push| Searcher { push });
        Ok(Search(results))
    }
}

#[guinea::test(iterations = 8)]
fn a_tokio_sleep_in_the_code_under_test_waits_on_the_test_clock(h: &mut Harness) {
    let started = std::time::Instant::now();

    h.install::<sleepy::Search>(&()).unwrap();
    h.dispatch::<Results>().emit(Query("gu".into()));

    h.settled();
    assert_eq!(h.state::<Results>().query, "", "answered before its sleep was over");
    assert_eq!(h.stuck(), 1);

    h.advance(std::time::Duration::from_millis(799));
    assert_eq!(h.state::<Results>().query, "", "a millisecond early");

    h.advance(std::time::Duration::from_millis(1));
    assert_eq!(h.state::<Results>().query, "gu");
    assert_eq!(h.stuck(), 0);

    assert!(started.elapsed() < std::time::Duration::from_millis(200), "waited in real time");
}

/// Samples on a timer, the way the example's metrics feature polls.
mod polling {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Samples {
        pub taken: u32,
    }

    #[derive(Clone, Debug)]
    pub struct Taken;

    #[reducer]
    fn samples(this: &mut Samples, _taken: Taken) {
        this.taken += 1;
    }

    #[derive(Clone, Debug)]
    pub struct Sample;

    #[derive(Debug)]
    pub struct Sampler {
        pub push: Push<Samples>,
    }

    actor! {
        Sampler {
            handlers { Sample }
        }
    }

    #[handler]
    fn sample(this: &mut Sampler, _: Sample) {
        this.push.send(Taken);
    }

    feature! {
        pub Polling {
            exports { Samples }
        }
    }

    #[installs]
    fn polling(cx: &FeatureInitContext) -> anyhow::Result<Polling> {
        let (samples, sampler) = cx.state::<Samples>().driven_by(|push| Sampler { push });
        cx.every(std::time::Duration::from_millis(100), || Sample, sampler);
        Ok(Polling(samples))
    }
}

#[guinea::test(iterations = 4)]
fn a_feature_timer_ticks_on_the_test_clock(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();

    h.advance(std::time::Duration::from_millis(99));
    assert_eq!(h.state::<polling::Samples>().taken, 0);

    h.advance(std::time::Duration::from_millis(901));
    assert_eq!(h.state::<polling::Samples>().taken, 10);
}

/// A poll that never ends runs beside the search. Waiting for everything to
/// go quiet would never return; waiting for what the query set off does, and
/// the chain it hands back says what that was.
#[guinea::test(iterations = 50)]
fn settling_one_action_waits_for_its_own_work_and_no_more(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();
    h.install::<sleepy::Search>(&()).unwrap();

    let asked = h.act::<Results>(Query("gu".into()));
    assert!(!asked.is_settled());

    asked.settle();
    assert_eq!(h.state::<Results>().query, "gu");

    let chain = asked.chain();
    assert!(chain.handled::<Query>());
    assert!(chain.handled::<Found>());
    assert!(chain.pushed::<Results>());
    assert!(!chain.cancelled());

    let taken = h.state::<polling::Samples>().taken;
    assert!(
        (7..=8).contains(&taken),
        "the clock went 800 ms for the search, and the poll ran on it: {taken}"
    );
}

/// The shape of what a query sets off, and where it leaves the state: a
/// refactor that drops the push, or sends the answer round an extra actor,
/// changes the first; one that reorders the results changes the second.
#[guinea::test(iterations = 50)]
fn what_a_query_sets_off_keeps_its_shape(h: &mut Harness) {
    h.install::<latest::Search>(&()).unwrap();

    let asked = h.act::<Results>(Query("guinea".into()));
    asked.settle();

    insta::assert_ron_snapshot!("a_query", asked.chain().shape());
    insta::assert_ron_snapshot!("its_results", *h.state::<Results>());
}

/// What an agent reports, arriving on the bus the way a report from
/// another process does.
mod reports {
    use super::*;

    #[derive(Clone, Debug, Event)]
    pub struct Report(pub Vec<&'static str>);

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Running {
        pub names: Vec<&'static str>,
    }

    #[reducer]
    fn running(this: &mut Running, report: Report) {
        this.names = report.0;
    }

    #[derive(Debug)]
    pub struct Reader {
        pub push: Push<Running>,
    }

    /// Asks for a report to be put on the bus, the way a page's
    /// End task asks the agent.
    #[derive(Clone, Debug)]
    pub struct Announce(pub Vec<&'static str>);

    actor! {
        Reader {
            handlers { Announce }
            publishes { Report }
            subscribes { Report }
        }
    }

    #[handler]
    fn report(this: &mut Reader, report: Report) {
        this.push.send(report);
    }

    #[handler]
    fn announce(_this: &mut Reader, Announce(text): Announce, cx: Cx) {
        cx.async_ctx().publish(Report(text));
    }

    feature! {
        pub Reports {
            exports { Running }
        }
    }

    #[installs]
    fn reports(cx: &FeatureInitContext) -> anyhow::Result<Reports> {
        let (running, _) = cx.state::<Running>().driven_by(|push| Reader { push });
        Ok(Reports(running))
    }

    /// What a plugin provides, for a feature to take.
    #[derive(Clone)]
    pub struct Prefix(pub &'static str);

    pub struct Prefixing;

    impl Plugin for Prefixing {
        const ID: &'static str = "test.prefixing";
        type Exports = ();
        type Provides = Prefix;

        fn build(self, _app: &mut PluginBuilder) -> anyhow::Result<Prefix> {
            Ok(Prefix("proc-"))
        }
    }

    thread_local! {
        static IN_PLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Puts something process-wide in place, the way the store plugin puts
    /// its global store, and refuses to do it twice.
    pub struct Global;

    impl Plugin for Global {
        const ID: &'static str = "test.global";
        type Exports = ();
        type Provides = ();

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            if IN_PLACE.get() {
                anyhow::bail!("a global is in place already");
            }

            IN_PLACE.set(true);
            app.on_cleanup(|| {
                IN_PLACE.set(false);
                Ok(())
            });
            Ok(())
        }
    }

    feature! {
        pub Prefixed {}
    }

    #[installs]
    fn prefixed(_cx: &FeatureInitContext, prefix: Prefix) -> anyhow::Result<Prefixed> {
        assert_eq!(prefix.0, "proc-");
        Ok(Prefixed)
    }

    /// A setting a feature has a default for and an application may override.
    #[derive(Clone, Copy, Default, Debug, PartialEq)]
    pub struct Limit(pub u32);

    thread_local! {
        pub static LIMITED_TO: std::cell::Cell<Option<Limit>> = const { std::cell::Cell::new(None) };
    }

    feature! {
        pub Limited {}
    }

    #[installs]
    fn limited(_cx: &FeatureInitContext, limit: Option<Limit>) -> anyhow::Result<Limited> {
        LIMITED_TO.set(Some(limit.unwrap_or_default()));
        Ok(Limited)
    }
}

#[guinea::test(iterations = 20)]
fn a_report_published_in_a_test_is_an_act_to_settle(h: &mut Harness) {
    h.install::<reports::Reports>(&()).unwrap();

    let first = h.publish(reports::Report(vec!["explorer", "code"]));
    first.settle();
    assert_eq!(h.state::<reports::Running>().names, ["explorer", "code"]);
    assert!(first.chain().handled::<reports::Report>());
    assert!(first.chain().pushed::<reports::Running>());

    h.publish(reports::Report(vec!["code"])).settle();
    assert_eq!(h.state::<reports::Running>().names, ["code"]);
}

/// A name a tool may set from outside, as an action or as the event the
/// action publishes.
mod named {
    use super::*;

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
    fn rename(_this: &mut Namer, Rename(name): Rename, cx: Cx) {
        cx.publish(Renamed(name));
    }

    #[handler]
    fn renamed(this: &mut Namer, renamed: Renamed) {
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
}

/// Another page's action with the same name as `named::Rename`, answered by a
/// feature of its own.
mod titled {
    use super::*;

    #[derive(Clone, Debug, serde::Deserialize, guinea::Remote)]
    #[remote(action)]
    pub struct Rename(pub String);

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Title(pub String);

    #[reducer]
    fn title(this: &mut Title, rename: Rename) {
        this.0 = rename.0;
    }

    #[derive(Debug)]
    pub struct Titler {
        pub push: Push<Title>,
    }

    actor! {
        Titler {
            handlers { Rename }
        }
    }

    #[handler]
    fn rename(this: &mut Titler, rename: Rename) {
        this.push.send(rename);
    }

    feature! {
        pub Titling {}
    }

    #[installs]
    fn titling(cx: &FeatureInitContext) -> anyhow::Result<Titling> {
        cx.state::<Title>().driven_by(|push| Titler { push });
        Ok(Titling)
    }
}

#[guinea::test(iterations = 4)]
fn an_action_sent_as_json_reaches_the_scope_that_answers_it(h: &mut Harness) {
    use guinea::core::remote;

    h.install::<named::Naming>(&()).unwrap();
    assert!(remote::actions().contains(&"Rename"), "{:?}", remote::actions());

    let scopes = [h.segment().scope()];

    remote::act_in(&scopes, "Rename", r#""guinea""#).unwrap();
    h.settled();
    assert_eq!(h.state::<named::Name>().0, "guinea");

    assert!(remote::act_in(&scopes, "Rename", "42").is_err(), "a number is not a name");
    assert!(remote::act_in(&scopes, "Rename", "42").unwrap_err().contains("Rename"));

    let tree = guinea::core::scope::ScopeTree::new();
    let elsewhere = [tree.scope()];
    let refused = remote::act_in(&elsewhere, "Rename", r#""x""#).unwrap_err();
    assert!(refused.contains("nothing on the open page answers"), "{refused}");
}

/// Two pages each have a `Rename`. The short name means the one the open
/// page answers, the page before the layout above it.
#[guinea::test(iterations = 4)]
fn two_actions_of_one_name_each_reach_the_page_that_answers_it(h: &mut Harness) {
    use guinea::core::remote;

    h.install::<named::Naming>(&()).unwrap();
    let page = h.child();
    page.install::<titled::Titling>(&()).unwrap();

    let layout = [h.segment().scope()];
    let both = [h.segment().scope(), page.scope()];

    remote::act_in(&both, "Rename", r#""page""#).unwrap();
    remote::act_in(&layout, "Rename", r#""layout""#).unwrap();
    h.settled();

    assert_eq!(page.state::<titled::Title>().0, "page");
    assert_eq!(h.state::<named::Name>().0, "layout");
}

/// Where both answer in one place, the short name is not enough - and the
/// path is.
#[guinea::test(iterations = 4)]
fn two_actions_of_one_name_in_one_scope_are_told_apart_by_path(h: &mut Harness) {
    use guinea::core::remote;

    h.install::<named::Naming>(&()).unwrap();
    h.install::<titled::Titling>(&()).unwrap();
    let scopes = [h.segment().scope()];

    let refused = remote::act_in(&scopes, "Rename", r#""x""#).unwrap_err();
    assert!(refused.contains("send one by its path"), "{refused}");
    assert!(refused.contains("titled::Rename"), "{refused}");

    remote::act_in(&scopes, "harness::titled::Rename", r#""by path""#).unwrap();
    h.settled();
    assert_eq!(h.state::<titled::Title>().0, "by path");
}

#[guinea::test(iterations = 4)]
fn an_event_sent_as_json_goes_out_on_the_application_s_bus(h: &mut Harness) {
    use guinea::core::remote;

    h.install::<named::Naming>(&()).unwrap();
    assert!(remote::events().contains(&"Renamed"), "{:?}", remote::events());

    remote::publish(h.application_scope(), "Renamed", r#""code""#).unwrap();
    h.settled();
    assert_eq!(h.state::<named::Name>().0, "code");
}

/// An actor that hears reports because its manifest says so.
mod listening {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Heard(pub u32);

    #[derive(Clone, Debug)]
    pub struct Once;

    #[reducer]
    fn heard(this: &mut Heard, _once: Once) {
        this.0 += 1;
    }

    #[derive(Debug)]
    pub struct Listener {
        pub push: Push<Heard>,
    }

    actor! {
        Listener {
            subscribes { reports::Report }
        }
    }

    #[handler]
    fn report(this: &mut Listener, _: reports::Report) {
        this.push.send(Once);
    }

    feature! {
        pub Listening {
            exports { Heard }
        }
    }

    #[installs]
    fn listening(cx: &FeatureInitContext) -> anyhow::Result<Listening> {
        let (heard, _) = cx.state::<Heard>().driven_by(|push| Listener { push });
        Ok(Listening(heard))
    }
}

#[guinea::test(iterations = 4)]
fn an_actor_hears_the_bus_until_its_page_is_left(h: &mut Harness) {
    let page = h.child();
    page.install::<reports::Reports>(&()).unwrap();
    assert_eq!(h.bus().subscriptions(), [("reports::Report", 1)]);

    page.leave();
    assert!(h.bus().subscriptions().is_empty(), "the actor went, and so did its subscription");
}

#[guinea::test(iterations = 1)]
fn a_callback_a_feature_subscribes_is_listed_where_the_feature_subscribed_it(h: &mut Harness) {
    use guinea::core::actor::event_bus::HeardBy;

    let page = h.child();
    let subscribed_at = line!() + 1;
    page.context().subscribe(|_: reports::Report| {});

    let listening = h.bus().listeners();
    let at = match listening[0].listeners[0].by {
        HeardBy::Callback(at) => at,
        ref other => panic!("a callback, not {other:?}"),
    };
    assert_eq!((at.file(), at.line()), (file!(), subscribed_at), "listed inside guinea instead");
}

#[guinea::test(iterations = 4)]
fn what_an_actor_s_manifest_subscribes_to_it_hears_and_lets_go_with_its_page(h: &mut Harness) {
    let page = h.child();
    page.install::<listening::Listening>(&()).unwrap();

    h.publish(reports::Report(vec!["code"])).settle();
    assert_eq!(page.state::<listening::Heard>().0, 1);

    page.leave();
    assert!(h.bus().subscriptions().is_empty(), "the manifest's subscription outlived the page");
}

/// The actor publishes through `AsyncContext::publish`, which hops to the
/// UI thread; settling the action waits for the hop and what it set off.
#[guinea::test(iterations = 20)]
fn settling_an_action_waits_for_what_it_published_on_the_application_s_bus(h: &mut Harness) {
    h.install::<reports::Reports>(&()).unwrap();

    let announced = h.act::<reports::Running>(reports::Announce(vec!["code"]));
    announced.settle();

    assert!(announced.chain().published::<reports::Report>(), "{:#?}", announced.chain());
    assert_eq!(h.state::<reports::Running>().names, ["code"]);
}

/// Each seed leaves a subscription behind on purpose; the next one must not
/// hear it.
#[guinea::test(iterations = 3)]
fn every_harness_starts_on_an_empty_bus(h: &mut Harness) {
    assert!(
        h.bus().subscriptions().is_empty(),
        "an earlier seed's subscribers are still on the bus"
    );

    h.install::<reports::Reports>(&()).unwrap();
    std::mem::forget(h.bus().subscribe_fn(|_: reports::Report| {}));

    assert_eq!(h.bus().subscriptions(), [("reports::Report", 2)]);
}

#[guinea::test(iterations = 2)]
fn what_a_plugin_hands_the_harness_serves_its_features_once_provided(h: &mut Harness) {
    let prefix = h.plugin(reports::Prefixing).unwrap();
    assert!(
        h.child().install::<reports::Prefixed>(&()).is_err(),
        "nothing provides the prefix yet"
    );

    h.provide(prefix);
    h.child().install::<reports::Prefixed>(&()).unwrap();
}

#[guinea::test(iterations = 2)]
fn a_service_provided_to_the_harness_serves_its_features(h: &mut Harness) {
    h.provide(reports::Prefix("proc-"));
    h.child().install::<reports::Prefixed>(&()).unwrap();
}

#[guinea::test(iterations = 2)]
fn a_setting_nobody_provided_is_its_default(h: &mut Harness) {
    h.child().install::<reports::Limited>(&()).unwrap();
    assert_eq!(reports::LIMITED_TO.get(), Some(reports::Limit(0)));
}

#[guinea::test(iterations = 2)]
fn a_setting_the_application_provided_is_what_it_provided(h: &mut Harness) {
    h.provide(reports::Limit(7));
    h.child().install::<reports::Limited>(&()).unwrap();
    assert_eq!(reports::LIMITED_TO.get(), Some(reports::Limit(7)));
}

/// How many tests are inside the one thing a process has one of.
static INSIDE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What a test holding a process-wide resource does: takes it, holds it a
/// while, lets it go - and would see another test in there with it.
fn hold_the_one_thing() {
    use std::sync::atomic::Ordering;

    assert_eq!(INSIDE.fetch_add(1, Ordering::SeqCst), 0, "another test is in here too");
    std::thread::sleep(std::time::Duration::from_millis(10));
    INSIDE.fetch_sub(1, Ordering::SeqCst);
}

#[guinea::test(iterations = 5, exclusive = "the one thing")]
fn tests_that_name_one_key_take_turns(_h: &mut Harness) {
    hold_the_one_thing();
}

#[guinea::test(iterations = 5, exclusive = "the one thing")]
fn tests_that_name_one_key_take_turns_with_this_one_too(_h: &mut Harness) {
    hold_the_one_thing();
}

/// Every seed installs the plugin again: the harness before it has to have
/// run the plugin's cleanup.
#[guinea::test(iterations = 3)]
fn a_harness_runs_its_plugins_cleanups_when_it_goes(h: &mut Harness) {
    h.plugin(reports::Global).unwrap();
}

/// A page's own count of what the layout above it sampled, kept by observing
/// the layout's reducer.
mod echo {
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Seen {
        pub samples: u32,
    }

    #[derive(Clone, Debug)]
    pub struct Saw;

    #[reducer]
    fn seen(this: &mut Seen, _saw: Saw) {
        this.samples += 1;
    }

    feature! {
        pub Echo {
            exports { Seen }
        }
    }

    #[installs]
    fn echo(cx: &FeatureInitContext) -> anyhow::Result<Echo> {
        let seen = cx.state::<Seen>().plain();

        let pushing = seen.clone();
        cx.observe::<polling::Samples>(move |_taken| pushing.push(Saw));

        Ok(Echo(seen))
    }
}

fn ms(millis: u64) -> std::time::Duration {
    std::time::Duration::from_millis(millis)
}

#[guinea::test(iterations = 8)]
fn a_page_reads_and_hears_what_the_layout_above_it_exports(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();

    let page = h.child();
    page.install::<echo::Echo>(&()).unwrap();

    h.advance(ms(300));
    assert_eq!(page.state::<polling::Samples>().taken, 3, "read from the layout");
    assert_eq!(page.state::<echo::Seen>().samples, 3, "heard every sample");
}

#[guinea::test(iterations = 50)]
fn leaving_a_page_mid_request_ends_its_work_and_only_its_work(h: &mut Harness) {
    h.install::<polling::Polling>(&()).unwrap();

    let page = h.child();
    page.install::<sleepy::Search>(&()).unwrap();

    let asked = page.act::<Results>(Query("gu".into()));
    h.advance(ms(400));
    page.leave();

    asked.settle();
    let chain = asked.chain();
    assert!(chain.cancelled(), "the search outlived its page:\n{chain:#?}");
    assert!(!chain.pushed::<Results>());

    h.advance(ms(600));
    assert_eq!(h.state::<polling::Samples>().taken, 10, "the layout's poll stopped with the page");
}

/// A watch the way a service pushes: nothing comes until the other end sends,
/// and the watch is opened by a click.
mod watching {
    use super::*;
    use guinea::core::__private::tokio::sync::mpsc;
    use guinea::core::actor::Stream;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    #[derive(Debug)]
    pub struct Changes(pub mpsc::UnboundedReceiver<u32>);

    impl Stream for Changes {
        type Item = u32;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<u32>> {
            self.0.poll_recv(cx)
        }
    }

    #[derive(Clone, Debug)]
    pub struct Watch(pub Arc<Mutex<Option<Changes>>>);

    impl Watch {
        pub fn of(changes: mpsc::UnboundedReceiver<u32>) -> Self {
            Self(Arc::new(Mutex::new(Some(Changes(changes)))))
        }
    }

    #[derive(Clone, Debug)]
    pub struct Changed(pub u32);

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Seen(pub Vec<u32>);

    #[reducer]
    fn seen(this: &mut Seen, Changed(value): Changed) {
        this.0.push(value);
    }

    #[derive(Debug)]
    pub struct Watcher {
        pub push: Push<Seen>,
    }

    actor! {
        Watcher {
            handlers { Watch => { task Changed }, Changed }
        }
    }

    #[handler]
    fn watch(_this: &mut Watcher, Watch(changes): Watch, cx: Cx) {
        let changes = changes.lock().unwrap().take().expect("a watch is opened once");
        cx.spawn_source(changes, Changed);
    }

    #[handler]
    fn changed(this: &mut Watcher, changed: Changed) {
        this.push.send(changed);
    }

    feature! {
        pub Watching {
            exports { Seen }
        }
    }

    #[installs]
    fn watching(cx: &FeatureInitContext) -> anyhow::Result<Watching> {
        let (seen, _) = cx.state::<Seen>().driven_by(|push| Watcher { push });
        Ok(Watching(seen))
    }
}

#[guinea::test(iterations = 20)]
fn a_click_that_opens_a_watch_is_done_once_the_watch_is_open(h: &mut Harness) {
    use guinea::core::__private::tokio::sync::mpsc;
    use guinea::core::trace::Point;

    h.install::<watching::Watching>(&()).unwrap();
    let (other_end, changes) = mpsc::unbounded_channel();

    let opened = h.act::<watching::Seen>(watching::Watch::of(changes));
    opened.settle();
    assert!(opened.chain().has(|point| matches!(point, Point::Source { .. })));

    other_end.send(1).unwrap();
    other_end.send(2).unwrap();
    h.settled();
    assert_eq!(h.state::<watching::Seen>().0, [1, 2]);
    assert!(
        !opened.chain().handled::<watching::Changed>(),
        "what the watch pushed is not the click's work:\n{:#?}",
        opened.chain()
    );

    drop(other_end);
    h.settled();
    assert!(
        opened.chain().has(|point| matches!(point, Point::Closed { gone: false, .. })),
        "the watch ran dry:\n{:#?}",
        opened.chain()
    );
    assert_eq!(h.stuck(), 0);
}

#[guinea::test(iterations = 20)]
fn leaving_the_page_closes_its_watch(h: &mut Harness) {
    use guinea::core::__private::tokio::sync::mpsc;
    use guinea::core::trace::Point;

    let page = h.child();
    page.install::<watching::Watching>(&()).unwrap();
    let (other_end, changes) = mpsc::unbounded_channel();

    let opened = page.act::<watching::Seen>(watching::Watch::of(changes));
    opened.settle();

    page.leave();
    h.settled();
    assert!(
        opened.chain().has(|point| matches!(point, Point::Closed { gone: true, .. })),
        "the watch outlived its page:\n{:#?}",
        opened.chain()
    );
    assert!(other_end.send(3).is_err(), "the watch let go of its end");
    assert_eq!(h.stuck(), 0);
}

/// An actor whose state counts itself: what is left of it once its page is
/// gone.
mod lingering {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        pub static ALIVE: Cell<usize> = const { Cell::new(0) };
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

    pub fn alive() -> usize {
        ALIVE.with(Cell::get)
    }

    #[derive(Clone, Debug)]
    pub struct Poke;

    #[derive(Clone, Debug)]
    pub struct Poked;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Pokes(pub u32);

    #[reducer]
    fn pokes(this: &mut Pokes, _poked: Poked) {
        this.0 += 1;
    }

    #[derive(Debug)]
    pub struct Lingerer {
        pub push: Push<Pokes>,
        pub _alive: Alive,
    }

    actor! {
        Lingerer {
            handlers { Poke => { task Poked }, Poked }
        }
    }

    #[handler]
    fn poke(_this: &mut Lingerer, _: Poke, cx: Cx) {
        cx.task().spawn(async { Poked });
    }

    #[handler]
    fn poked(this: &mut Lingerer, poked: Poked) {
        this.push.send(poked);
    }

    feature! {
        pub Lingering {
            exports { Pokes }
        }
    }

    #[installs]
    fn lingering(cx: &FeatureInitContext) -> anyhow::Result<Lingering> {
        let (pokes, addr) = cx.state::<Pokes>().driven_by(|push| Lingerer { push, _alive: Alive::new() });
        cx.every(ms(100), || Poke, addr);
        Ok(Lingering(pokes))
    }
}

#[guinea::test(iterations = 8)]
fn an_actor_is_let_go_of_with_its_page(h: &mut Harness) {
    let page = h.child();
    page.install::<lingering::Lingering>(&()).unwrap();
    assert_eq!(lingering::alive(), 1);

    page.leave();
    h.settled();

    assert_eq!(lingering::alive(), 0, "the actor outlived its page");
}

#[guinea::test(iterations = 8)]
fn an_actor_that_worked_is_let_go_of_with_its_page(h: &mut Harness) {
    let page = h.child();
    page.install::<lingering::Lingering>(&()).unwrap();
    page.act::<lingering::Pokes>(lingering::Poke).settle();
    h.advance(ms(250));
    assert!(page.state::<lingering::Pokes>().0 >= 3);

    page.leave();
    h.settled();

    assert_eq!(lingering::alive(), 0, "the actor outlived its page");
}

/// A background answer whose handler starts an actor and ends one - what a
/// supervisor does when an agent comes and goes.
mod supervising {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct Ping;

    #[derive(Debug)]
    pub struct Agent;

    actor! {
        Agent {
            handlers { Ping }
        }
    }

    #[handler]
    fn ping(_this: &mut Agent, _: Ping) {}

    #[derive(Clone, Debug)]
    pub struct Look;

    #[derive(Clone, Debug)]
    pub struct Found;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Agents(pub u32);

    #[reducer]
    fn agents(this: &mut Agents, _found: Found) {
        this.0 += 1;
    }

    #[derive(Debug)]
    pub struct Supervisor {
        pub push: Push<Agents>,
    }

    actor! {
        Supervisor {
            handlers { Look => { task Found }, Found }
        }
    }

    #[handler]
    fn look(_this: &mut Supervisor, _: Look, cx: Cx) {
        cx.task().spawn(async { Found });
    }

    #[handler]
    fn found(this: &mut Supervisor, found: Found, cx: Cx) {
        let comes_and_goes = cx.addr().home().child();
        let agent = Addr::new(Agent, &comes_and_goes);
        agent.send(Ping);
        comes_and_goes.scope().remove();

        this.push.send(found);
    }

    feature! {
        pub Supervising {
            exports { Agents }
        }
    }

    #[installs]
    fn supervising(cx: &FeatureInitContext) -> anyhow::Result<Supervising> {
        let (agents, _) = cx.state::<Agents>().driven_by(|push| Supervisor { push });
        Ok(Supervising(agents))
    }
}

/// A feature's actor that makes another actor after the feature installed.
mod hiring {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct Hire;

    #[derive(Debug)]
    pub struct Worker;

    actor! {
        Worker {
            handlers { Hire }
        }
    }

    #[handler]
    fn work(_this: &mut Worker, _: Hire) {}

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Staff(pub u32);

    #[reducer]
    fn staff(this: &mut Staff, staff: u32) {
        this.0 = staff;
    }

    #[derive(Debug)]
    pub struct Boss {
        pub push: Push<Staff>,
    }

    actor! {
        Boss {
            handlers { Hire }
        }
    }

    #[handler]
    fn hire(this: &mut Boss, _: Hire, cx: Cx) {
        let _worker = Addr::new(Worker, &cx.addr().home());
        this.push.send(1);
    }

    feature! {
        pub Hiring {
            exports { Staff }
        }
    }

    #[installs]
    fn hiring(cx: &FeatureInitContext) -> anyhow::Result<Hiring> {
        let (staff, _) = cx.state::<Staff>().driven_by(|push| Boss { push });
        Ok(Hiring(staff))
    }
}

#[guinea::test(iterations = 1)]
fn an_actor_a_feature_s_actor_makes_later_is_the_feature_s(h: &mut Harness) {
    h.install::<hiring::Hiring>(&()).unwrap();
    h.act::<hiring::Staff>(hiring::Hire).settle();

    let actors = h.segment().scope().actors_here();
    let feature_of = |actor: &str| {
        actors
            .iter()
            .find(|listed| listed.type_name.ends_with(actor))
            .map(|listed| listed.owner.feature)
    };

    assert_eq!(feature_of("Worker"), feature_of("Boss"));
    assert!(feature_of("Boss").flatten().is_some());
}

#[guinea::test(iterations = 8)]
fn a_background_answer_may_start_and_end_actors(h: &mut Harness) {
    h.install::<supervising::Supervising>(&()).unwrap();

    h.act::<supervising::Agents>(supervising::Look).settle();
    assert_eq!(h.state::<supervising::Agents>().0, 1);
}

#[test]
fn a_seed_is_one_order_every_time() {
    let shown = |seed| {
        let h = Harness::new(seed);
        h.install::<naive::Search>(&()).unwrap();
        type_gu_then_guinea(&h);
        h.state::<Results>().query.clone()
    };

    let first: Vec<String> = (0..64).map(shown).collect();
    let again: Vec<String> = (0..64).map(shown).collect();
    assert_eq!(first, again);

    assert!(
        first.iter().any(|query| query == "gu") && first.iter().any(|query| query == "guinea"),
        "the seeds reach both orders: {first:?}"
    );
}

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .unwrap_or_default()
}

#[guinea::test(iterations = 1)]
fn a_harness_settles_again_after_a_task_panicked(h: &mut Harness) {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    guinea::core::executor::spawn(async { panic!("the task under test blew up") });
    let first = catch_unwind(AssertUnwindSafe(|| h.settled())).map_err(said);
    assert!(matches!(&first, Err(said) if said.contains("blew up")), "{first:?}");

    let ran = Arc::new(AtomicBool::new(false));
    let marked = ran.clone();
    guinea::core::executor::spawn(async move { marked.store(true, Ordering::SeqCst) });
    let second = catch_unwind(AssertUnwindSafe(|| h.settled())).map_err(said);

    assert_eq!(second, Ok(1), "settling the next task");
    assert!(ran.load(Ordering::SeqCst));
}

mod forever {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Waiting(pub u32);

    #[derive(Clone, Debug)]
    pub struct Wait;

    #[derive(Clone, Debug)]
    pub struct Never;

    #[reducer]
    fn waiting(this: &mut Waiting, _never: Never) {
        this.0 += 1;
    }

    #[derive(Debug)]
    pub struct Waiter {
        pub push: Push<Waiting>,
    }

    actor! {
        Waiter {
            handlers { Wait => { task Never }, Never }
        }
    }

    #[handler]
    fn wait(_this: &mut Waiter, _: Wait, cx: Cx) {
        cx.task().spawn(async {
            std::future::pending::<()>().await;
            Never
        });
    }

    #[handler]
    fn never(this: &mut Waiter, never: Never) {
        this.push.send(never);
    }

    thread_local! {
        pub static TICKS: Cell<u32> = const { Cell::new(0) };
    }

    feature! {
        pub Forever {
            exports { Waiting }
        }
    }

    #[installs]
    fn forever(cx: &FeatureInitContext) -> anyhow::Result<Forever> {
        let (waiting, _) = cx.state::<Waiting>().driven_by(|push| Waiter { push });
        cx.repeat(Duration::from_secs(60 * 60), || {
            let ticks = TICKS.get() + 1;
            TICKS.set(ticks);
            if ticks > 48 {
                panic!("an hourly timer nothing waits on moved the clock two days on");
            }
        });
        Ok(Forever(waiting))
    }
}

#[guinea::test(iterations = 1)]
fn settling_an_action_that_waits_on_nothing_says_so_beside_an_unrelated_timer(h: &mut Harness) {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    forever::TICKS.set(0);
    h.install::<forever::Forever>(&()).unwrap();

    let waiting = h.act::<forever::Waiting>(forever::Wait);
    let settled = catch_unwind(AssertUnwindSafe(|| {
        waiting.settle();
    }))
    .map_err(said);

    assert!(
        matches!(&settled, Err(said) if said.contains("never finished")),
        "settle gave {settled:?} after {} hourly ticks",
        forever::TICKS.get()
    );
}

#[guinea::test(iterations = 1)]
fn a_task_ready_to_run_is_not_stuck(h: &mut Harness) {
    guinea::core::executor::spawn(async {});

    assert_eq!(h.stuck(), 0, "a task nothing has polled yet is ready, not waiting");
}
