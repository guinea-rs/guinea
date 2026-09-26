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

impl Reducer for Results {
    type Update = Found;

    fn reduce(&mut self, update: Found) {
        self.query = update.query;
        self.found = update.found;
    }
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
            handlers { Query => { bg Found }, Found }
        }
    }

    #[handler]
    fn query(_this: &mut Searcher, ctx: Context<Searcher, Query>) {
        let text = ctx.msg.0.clone();
        ctx.spawn_bg::<Found, _>(search(text));
    }

    #[handler]
    fn found(this: &mut Searcher, ctx: Context<Searcher, Found>) {
        this.push.send(ctx.msg.clone());
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
            handlers { Query => { bg Answered }, Answered }
        }
    }

    #[handler]
    fn query(this: &mut Searcher, ctx: Context<Searcher, Query>) {
        this.asked += 1;

        let asked = this.asked;
        let text = ctx.msg.0.clone();
        ctx.spawn_bg::<Answered, _>(async move {
            Answered {
                asked,
                found: search(text).await,
            }
        });
    }

    #[handler]
    fn answered(this: &mut Searcher, ctx: Context<Searcher, Answered>) {
        if ctx.msg.asked == this.asked {
            this.push.send(ctx.msg.found.clone());
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
            handlers { Query => { bg Found }, Found }
        }
    }

    #[handler]
    fn query(_this: &mut Searcher, ctx: Context<Searcher, Query>) {
        let text = ctx.msg.0.clone();
        ctx.spawn_bg::<Found, _>(async move {
            guinea::core::__private::tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            Found { query: text, found: Vec::new() }
        });
    }

    #[handler]
    fn found(this: &mut Searcher, ctx: Context<Searcher, Found>) {
        this.push.send(ctx.msg.clone());
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

    impl Reducer for Samples {
        type Update = Taken;

        fn reduce(&mut self, _taken: Taken) {
            self.taken += 1;
        }
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
    fn sample(this: &mut Sampler, _ctx: Context<Sampler, Sample>) {
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
        cx.every(std::time::Duration::from_millis(100), &sampler, || Sample);
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

/// What an agent reports, arriving on the global bus the way a report from
/// another process does.
mod reports {
    use super::*;

    #[derive(Clone, Debug, Event)]
    pub struct Report(pub Vec<&'static str>);

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Running {
        pub names: Vec<&'static str>,
    }

    impl Reducer for Running {
        type Update = Report;

        fn reduce(&mut self, report: Report) {
            self.names = report.0;
        }
    }

    #[derive(Debug)]
    pub struct Reader {
        pub push: Push<Running>,
    }

    /// Asks for a report to be put on the global bus, the way a page's
    /// End task asks the agent.
    #[derive(Clone, Debug)]
    pub struct Announce(pub Vec<&'static str>);

    actor! {
        Reader {
            handlers { Report, Announce }
        }
    }

    #[handler]
    fn report(this: &mut Reader, ctx: Context<Reader, Report>) {
        this.push.send(ctx.msg.clone());
    }

    #[handler]
    fn announce(_this: &mut Reader, ctx: Context<Reader, Announce>) {
        GlobalEventBus::publish(Report(ctx.msg.0.clone()));
    }

    feature! {
        pub Reports {
            exports { Running }
        }
    }

    #[installs]
    fn reports(cx: &FeatureInitContext) -> anyhow::Result<Reports> {
        let (running, reader) = cx.state::<Running>().driven_by(|push| Reader { push });
        reader.subscribe_on::<Report>(Bus::Global);
        Ok(Reports(running))
    }

    /// What a plugin provides, for a feature to read.
    pub struct Prefix(pub &'static str);

    pub struct Prefixing;

    impl Plugin for Prefixing {
        const ID: &'static str = "test.prefixing";

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            app.provide(Prefix("proc-"));
            Ok(())
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

        fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
            if IN_PLACE.get() {
                anyhow::bail!("a global is in place already");
            }

            IN_PLACE.set(true);
            app.on_cleanup(|_| {
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
    fn prefixed(cx: &FeatureInitContext) -> anyhow::Result<Prefixed> {
        let prefix = cx.require::<Prefix>()?;
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
    fn limited(cx: &FeatureInitContext) -> anyhow::Result<Limited> {
        LIMITED_TO.set(Some(cx.require_or_default::<Limit>()));
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

    impl Reducer for Name {
        type Update = Renamed;

        fn reduce(&mut self, renamed: Renamed) {
            self.0 = renamed.0;
        }
    }

    #[derive(Debug)]
    pub struct Namer {
        pub push: Push<Name>,
    }

    actor! {
        Namer {
            handlers { Rename, Renamed }
        }
    }

    #[handler]
    fn rename(_this: &mut Namer, ctx: Context<Namer, Rename>) {
        GlobalEventBus::publish(Renamed(ctx.msg.0.clone()));
    }

    #[handler]
    fn renamed(this: &mut Namer, ctx: Context<Namer, Renamed>) {
        this.push.send(ctx.msg.clone());
    }

    feature! {
        pub Naming {
            exports { Name }
        }
    }

    #[installs]
    fn naming(cx: &FeatureInitContext) -> anyhow::Result<Naming> {
        let (name, namer) = cx.state::<Name>().driven_by(|push| Namer { push });
        namer.subscribe_on::<Renamed>(Bus::Global);
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

    impl Reducer for Title {
        type Update = Rename;

        fn reduce(&mut self, rename: Rename) {
            self.0 = rename.0;
        }
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
    fn rename(this: &mut Titler, ctx: Context<Titler, Rename>) {
        this.push.send(ctx.msg.clone());
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

    let scopes = [h.segment().context().scope.clone()];

    remote::act_in(&scopes, "Rename", r#""guinea""#).unwrap();
    h.settled();
    assert_eq!(h.state::<named::Name>().0, "guinea");

    assert!(remote::act_in(&scopes, "Rename", "42").is_err(), "a number is not a name");
    assert!(remote::act_in(&scopes, "Rename", "42").unwrap_err().contains("Rename"));

    let elsewhere = [std::rc::Rc::new(guinea::core::scope::Scope::new())];
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

    let layout = [h.segment().context().scope.clone()];
    let both = [h.segment().context().scope.clone(), page.context().scope.clone()];

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
    let scopes = [h.segment().context().scope.clone()];

    let refused = remote::act_in(&scopes, "Rename", r#""x""#).unwrap_err();
    assert!(refused.contains("send one by its path"), "{refused}");
    assert!(refused.contains("titled::Rename"), "{refused}");

    remote::act_in(&scopes, "harness::titled::Rename", r#""by path""#).unwrap();
    h.settled();
    assert_eq!(h.state::<titled::Title>().0, "by path");
}

#[guinea::test(iterations = 4)]
fn an_event_sent_as_json_goes_out_on_the_global_bus(h: &mut Harness) {
    use guinea::core::remote;

    h.install::<named::Naming>(&()).unwrap();
    assert!(remote::events().contains(&"Renamed"), "{:?}", remote::events());

    remote::publish("Renamed", r#""code""#).unwrap();
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

    impl Reducer for Heard {
        type Update = Once;

        fn reduce(&mut self, _once: Once) {
            self.0 += 1;
        }
    }

    #[derive(Debug)]
    pub struct Listener {
        pub push: Push<Heard>,
    }

    actor! {
        Listener {
            handlers { reports::Report }
            subscribes { reports::Report }
        }
    }

    #[handler]
    fn report(this: &mut Listener, _ctx: Context<Listener, reports::Report>) {
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
    assert_eq!(GlobalEventBus::bus().subscriptions(), [("reports::Report", 1)]);

    page.leave();
    assert!(GlobalEventBus::bus().subscriptions().is_empty(), "the actor went, and so did its subscription");
}

#[guinea::test(iterations = 4)]
fn what_an_actor_s_manifest_subscribes_to_it_hears_and_lets_go_with_its_page(h: &mut Harness) {
    let page = h.child();
    page.install::<listening::Listening>(&()).unwrap();

    h.publish(reports::Report(vec!["code"])).settle();
    assert_eq!(page.state::<listening::Heard>().0, 1);

    page.leave();
    assert!(GlobalEventBus::bus().subscriptions().is_empty(), "the manifest's subscription outlived the page");
}

/// The actor publishes through `GlobalEventBus::publish`, which hops to the
/// UI thread; settling the action waits for the hop and what it set off.
#[guinea::test(iterations = 20)]
fn settling_an_action_waits_for_what_it_published_on_the_global_bus(h: &mut Harness) {
    h.install::<reports::Reports>(&()).unwrap();

    let announced = h.act::<reports::Running>(reports::Announce(vec!["code"]));
    announced.settle();

    assert!(announced.chain().published::<reports::Report>(), "{:#?}", announced.chain());
    assert_eq!(h.state::<reports::Running>().names, ["code"]);
}

/// Each seed leaves a subscription behind on purpose; the next one must not
/// hear it.
#[guinea::test(iterations = 3)]
fn every_harness_starts_on_an_empty_global_bus(h: &mut Harness) {
    assert!(
        GlobalEventBus::bus().subscriptions().is_empty(),
        "an earlier seed's subscribers are still on the bus"
    );

    h.install::<reports::Reports>(&()).unwrap();
    std::mem::forget(GlobalEventBus::subscribe_fn(|_: reports::Report| {}));

    assert_eq!(GlobalEventBus::bus().subscriptions(), [("reports::Report", 2)]);
}

#[guinea::test(iterations = 2)]
fn a_plugin_installed_into_the_harness_serves_its_features(h: &mut Harness) {
    assert!(
        h.child().install::<reports::Prefixed>(&()).is_err(),
        "nothing provides the prefix yet"
    );

    h.plugin(reports::Prefixing).unwrap();
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

    impl Reducer for Seen {
        type Update = Saw;

        fn reduce(&mut self, _saw: Saw) {
            self.samples += 1;
        }
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
