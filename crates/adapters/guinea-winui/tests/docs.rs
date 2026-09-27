#![cfg(windows)]
#![allow(dead_code)]

//! The examples in the documentation of `Page` and `Layout`, compiled and run.
//! `cargo xtask docs` copies what is between the marks into the doc comments.

mod only_a_view {
    //@show a page that is only a view
    use guinea_winui::{Page, PageCx, page};
    use windows_reactor::{TextBlock, View};

    #[derive(Default)]
    struct About;

    #[page]
    impl Page for About {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("guinea").into()
        }
    }
    //@show-end
}

mod whole {
    //@show a whole page
    //@hide
    use guinea_app::feature::Segment;
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
    use windows_reactor::{Button, ChildrenControl, ContentControl, StackPanel, TextBlock, View};

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Count(pub u32);

    #[reducer]
    fn count(this: &mut Count, by: u32) {
        this.0 += by;
    }

    /// What the page asks the feature for.
    pub struct Add(pub u32);

    feature! {
        pub Counter {
            exports { Count }
        }
    }

    #[installs]
    fn counter(cx: &FeatureInitContext) -> anyhow::Result<Counter> {
        let count = cx.state::<Count>().plain();

        let adding = count.clone();
        cx.answers(move |Add(by): Add| adding.push(by));

        Ok(Counter(count))
    }

    pub struct CounterPage {
        step: u32,
    }

    impl Default for CounterPage {
        fn default() -> Self {
            Self { step: 1 }
        }
    }

    pub enum Msg {
        Bigger,
    }

    #[page]
    impl Page for CounterPage {
        type Installs = Counter;
        type Message = Msg;

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Counter> {
            ctx.install(&())
        }

        fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Bigger => self.step += 1,
            }
        }

        fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
            let (count, dispatch) = cx.use_reducer::<Count, _>();
            let step = self.step;

            StackPanel::new()
                .children((
                    TextBlock::new().text(format!("{} by {step}", count.0)),
                    Button::new()
                        .on_click(move || dispatch.emit(Add(step)))
                        .content(TextBlock::new().text("Add")),
                    Button::new()
                        .on_click(cx.on(|()| Msg::Bigger))
                        .content(TextBlock::new().text("Bigger")),
                ))
                .into()
        }
    }

    // `routes!` writes where each segment sits; this page stands alone.
    impl Segment for CounterPage {
        type Installs = Counter;
        type Above = ();
    }

    // Under test it mounts with no window, in a harness that runs what it
    // sets off in a seeded order - `harness::Mounted`, behind the `harness`
    // feature. `#[guinea::test]` is this `check` as an attribute.
    //@hide
    #[cfg(feature = "harness")]
    //@unhide
    #[test]
    fn adds_the_step_it_was_told() {
        guinea_app::app::check(4, |h| {
            let mut page =
                guinea_winui::harness::Mounted::<CounterPage>::mount(&h.segment(), ()).unwrap();

            page.click_text("Bigger").settle();
            page.click_text("Add").settle();
            page.settle();

            assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
        });
    }
    //@show-end
}

mod cached {
    //@show a page whose state outlives it
    use guinea_winui::{Page, PageCx, page};
    use windows_reactor::{TextBlock, View};

    #[derive(Default)]
    struct Processes;

    #[page]
    impl Page for Processes {
        // Back from another tab, the list is there at once rather than
        // empty until the next refresh arrives.
        const CACHE_STATE_IN_MEMORY: bool = true;

        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("processes").into()
        }
    }
    //@show-end
}

mod installing {
    //@show a page that installs
    //@hide
    use guinea_app::feature::Segment;
    use guinea_core::feature::Bound;
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{FeatureInitContext, Page, PageCx, page};
    use windows_reactor::{TextBlock, View};

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Listing(pub String);

    #[reducer]
    fn listing(this: &mut Listing, to: String) {
        this.0 = to;
    }

    /// Which row is selected: the page's own, with no feature around it.
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Selection(pub Option<u32>);

    #[reducer]
    fn selection(this: &mut Selection, to: Option<u32>) {
        this.0 = to;
    }

    feature! {
        pub Processes {
            exports { Listing }
        }
    }

    /// A feature with parameters: which machine to list.
    #[installs]
    fn processes(cx: &FeatureInitContext, context: &str) -> anyhow::Result<Processes> {
        let listing = cx.state::<Listing>().plain();
        listing.push(format!("processes on {context}"));
        Ok(Processes(listing))
    }

    #[derive(PartialEq)]
    pub struct ProcessesParams {
        pub context: String,
    }

    #[derive(Default)]
    pub struct ProcessesPage;

    #[page]
    impl Page for ProcessesPage {
        type Params = ProcessesParams;
        type Installs = (Processes, Bound<Selection>);

        fn install(
            ctx: &FeatureInitContext,
            params: &ProcessesParams,
        ) -> anyhow::Result<Self::Installs> {
            let processes = ctx.install::<Processes>(&params.context)?;
            let selection = ctx.state::<Selection>().plain();
            Ok((processes, selection))
        }

        fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
            let (listing, _) = cx.use_reducer::<Listing, _>();
            TextBlock::new().text(listing.0.clone()).into()
        }
    }
    //@hide

    impl Segment for ProcessesPage {
        type Installs = <ProcessesPage as Page>::Installs;
        type Above = ();
    }

    #[cfg(feature = "harness")]
    #[test]
    fn lists_the_machine_it_was_reached_with() {
        guinea_app::app::check(2, |h| {
            let params = ProcessesParams {
                context: "ubuntu".into(),
            };
            let mut page =
                guinea_winui::harness::Mounted::<ProcessesPage>::mount(&h.segment(), params)
                    .unwrap();
            page.settle();
            assert!(
                page.find_text("processes on ubuntu").is_some(),
                "{:#?}",
                page.tree()
            );
        });
    }
    //@unhide
    //@show-end
}

mod keeping_the_capture {
    //@show a page that keeps its capture
    //@hide
    use guinea_app::feature::Segment;
    //@unhide
    use guinea_winui::{FeatureInitContext, Page, PageCx, page};
    use windows_reactor::{TextBlock, View};

    #[derive(PartialEq)]
    pub struct ProcessParams {
        pub pid: u32,
    }

    #[derive(Default)]
    pub struct ProcessPage {
        pid: u32,
    }

    #[page]
    impl Page for ProcessPage {
        type Params = ProcessParams;

        fn init(_ctx: &FeatureInitContext, params: &ProcessParams) -> Self {
            Self { pid: params.pid }
        }

        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new()
                .text(format!("process {}", self.pid))
                .into()
        }
    }
    //@hide

    impl Segment for ProcessPage {
        type Installs = ();
        type Above = ();
    }

    #[cfg(feature = "harness")]
    #[test]
    fn shows_the_pid_it_was_reached_with() {
        guinea_app::app::check(2, |h| {
            let page = guinea_winui::harness::Mounted::<ProcessPage>::mount(
                &h.segment(),
                ProcessParams { pid: 42 },
            )
            .unwrap();
            assert!(page.find_text("process 42").is_some(), "{:#?}", page.tree());
        });
    }
    //@unhide
    //@show-end
}

mod minding_the_page {
    //@show a page that minds being left
    use guinea_winui::{Ask, Page, PageCx, Verdict, page};
    use windows_reactor::{TextBlock, View};

    #[derive(Default)]
    pub struct Editor {
        text: String,
        saved: String,
    }

    #[page]
    impl Page for Editor {
        fn leaving(&self) -> Verdict {
            if self.text == self.saved {
                Verdict::Allow
            } else {
                Verdict::ask(Ask::new("Discard the changes?", "Discard", "Stay"))
            }
        }

        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text(self.text.clone()).into()
        }
    }

    #[test]
    fn asks_only_with_changes() {
        let mut editor = Editor::default();
        assert!(matches!(editor.leaving(), Verdict::Allow));

        editor.text = "draft".into();
        assert!(matches!(editor.leaving(), Verdict::Ask(..)));
    }
    //@show-end
}

mod asking_a_feature {
    //@show a page that asks a feature
    //@hide
    use guinea_app::feature::Segment;
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
    use windows_reactor::{TextBlock, View};

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Results(pub String);

    #[reducer]
    fn results(this: &mut Results, to: String) {
        this.0 = to;
    }

    pub struct Search(pub String);

    feature! {
        pub Searching {
            exports { Results }
        }
    }

    #[installs]
    fn searching(cx: &FeatureInitContext) -> anyhow::Result<Searching> {
        let results = cx.state::<Results>().plain();

        let answering = results.clone();
        cx.answers(move |Search(query): Search| answering.push(format!("found {query}")));

        Ok(Searching(results))
    }

    #[derive(Default)]
    pub struct SearchPage {
        query: String,
    }

    pub enum Msg {
        Typed(String),
        Submitted,
    }

    #[page]
    impl Page for SearchPage {
        type Installs = Searching;
        type Message = Msg;

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Searching> {
            ctx.install(&())
        }

        fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Typed(text) => self.query = text,
                Msg::Submitted => {
                    let (_, search) = cx.state::<Results, _>();
                    search.emit(Search(self.query.clone()));
                }
            }
        }

        fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
            let (results, _) = cx.use_reducer::<Results, _>();
            TextBlock::new().text(results.0.clone()).into()
        }
    }
    //@hide

    impl Segment for SearchPage {
        type Installs = Searching;
        type Above = ();
    }

    #[cfg(feature = "harness")]
    #[test]
    fn submitting_asks_for_what_was_typed() {
        guinea_app::app::check(2, |h| {
            let mut page =
                guinea_winui::harness::Mounted::<SearchPage>::mount(&h.segment(), ()).unwrap();
            page.send(Msg::Typed("gu".into()));
            page.send(Msg::Submitted);
            page.settle();
            assert!(page.find_text("found gu").is_some(), "{:#?}", page.tree());
        });
    }
    //@unhide
    //@show-end
}

mod answering_a_widget {
    //@show a page that answers a widget
    //@hide
    use guinea_app::feature::Segment;
    //@unhide
    use guinea_winui::{Page, PageCx, UpdateCx, page};
    use windows_reactor::{ChildrenControl, StackPanel, TextBlock, TextBox, View};

    #[derive(Default)]
    pub struct Greeting {
        name: String,
    }

    pub enum Msg {
        Named(String),
    }

    #[page]
    impl Page for Greeting {
        type Message = Msg;

        fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Named(name) => self.name = name,
            }
        }

        fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
            StackPanel::new()
                .children((
                    TextBox::new()
                        .text(self.name.clone())
                        .on_text_changed(cx.on(Msg::Named)),
                    TextBlock::new().text(format!("hello, {}", self.name)),
                ))
                .into()
        }
    }
    //@hide

    impl Segment for Greeting {
        type Installs = ();
        type Above = ();
    }

    #[cfg(feature = "harness")]
    #[test]
    fn greets_whoever_was_typed() {
        guinea_app::app::check(2, |h| {
            let mut page =
                guinea_winui::harness::Mounted::<Greeting>::mount(&h.segment(), ()).unwrap();
            page.send(Msg::Named("guinea".into()));
            page.settle();
            assert!(
                page.find_text("hello, guinea").is_some(),
                "{:#?}",
                page.tree()
            );
        });
    }
    //@unhide
    //@show-end
}

mod shell {
    //@show a shell
    //@hide
    use guinea_app::feature::Segment;
    use guinea_core::feature::Bound;
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{
        FeatureInitContext, Layout, LayoutCx, Page, PageCx, UpdateCx, layout, page,
    };
    use windows_reactor::{Button, ChildrenControl, ContentControl, StackPanel, TextBlock, View};

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Sidebar {
        pub open: bool,
    }

    #[reducer]
    fn sidebar(this: &mut Sidebar, open: bool) {
        this.open = open;
    }

    pub struct SetOpen(pub bool);

    feature! {
        pub Chrome {
            exports { Sidebar }
        }
    }

    #[installs]
    fn chrome(cx: &FeatureInitContext) -> anyhow::Result<Chrome> {
        let sidebar = cx.state::<Sidebar>().seed(Sidebar { open: true }).plain();

        let setting = sidebar.clone();
        cx.answers(move |SetOpen(open): SetOpen| setting.push(open));

        Ok(Chrome(sidebar))
    }

    /// State the shell keeps itself, with no feature around it.
    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Title(pub String);

    #[reducer]
    fn title(this: &mut Title, title: String) {
        this.0 = title;
    }

    #[derive(Default)]
    pub struct Home;

    #[page]
    impl Page for Home {
        fn view(&self, cx: &mut PageCx<'_, Self>) -> View {
            // Installed by the shell above, and readable here because `Chrome`
            // exports it. A page outside the shell asking for it does not
            // compile.
            let (sidebar, _) = cx.use_reducer::<Sidebar, _>();
            let width = if sidebar.open { "narrow" } else { "wide" };

            TextBlock::new().text(format!("home, {width}")).into()
        }
    }

    #[derive(Default)]
    pub struct Shell;

    pub enum ShellMsg {
        Toggle,
    }

    #[layout]
    impl Layout for Shell {
        type Params = ();
        /// A flat list: a feature, and a reducer claimed directly. Pages below
        /// may read `Sidebar`, which `Chrome` exports, and `Title`.
        type Installs = (Chrome, Bound<Title>);
        type Message = ShellMsg;

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self::Installs> {
            let title = ctx.state::<Title>().seed(Title("guinea".into())).plain();
            Ok((ctx.install(&())?, title))
        }

        fn update(&mut self, message: ShellMsg, cx: &mut UpdateCx<'_, Self>) {
            match message {
                ShellMsg::Toggle => {
                    let (sidebar, dispatch) = cx.state::<Sidebar, _>();
                    dispatch.emit(SetOpen(!sidebar.open));
                }
            }
        }

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            let (sidebar, _) = cx.use_reducer::<Sidebar, _>();
            let (title, _) = cx.use_reducer::<Title, _>();

            let tab = if cx.child_is::<Home>() {
                "> Home"
            } else {
                "Home"
            };
            let side: View = if sidebar.open {
                TextBlock::new().text(tab).into()
            } else {
                View::empty()
            };

            StackPanel::new()
                .children((
                    TextBlock::new().text(title.0.clone()),
                    Button::new()
                        .on_click(cx.on(|()| ShellMsg::Toggle))
                        .content(TextBlock::new().text("Menu")),
                    side,
                    cx.outlet(),
                ))
                .into()
        }
    }

    // Where each segment sits, which is what `routes!` writes.
    impl Segment for Shell {
        type Installs = <Shell as Layout>::Installs;
        type Above = ();
    }

    impl Segment for Home {
        type Installs = ();
        type Above = (Shell, ());
    }
    //@show-end
}

mod per_context {
    //@show a layout per context
    //@hide
    use guinea_core::scope::Reducer;
    use guinea_macros::{feature, installs};
    //@unhide
    use guinea_winui::{FeatureInitContext, Layout, LayoutCx, layout};
    use windows_reactor::View;
    //@hide

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Machine(pub String);

    impl Reducer for Machine {
        type Update = String;

        fn reduce(&mut self, to: String) {
            self.0 = to;
        }
    }

    feature! {
        pub Connection {
            exports { Machine }
        }
    }

    #[installs]
    fn connection(cx: &FeatureInitContext, context: &str) -> anyhow::Result<Connection> {
        let machine = cx.state::<Machine>().plain();
        machine.push(context.to_string());
        Ok(Connection(machine))
    }
    //@unhide

    /// What `routes!` writes when every page below captures `context`.
    #[derive(PartialEq)]
    pub struct MachineParams {
        pub context: String,
    }

    #[derive(Default)]
    pub struct MachineLayout;

    #[layout]
    impl Layout for MachineLayout {
        type Params = MachineParams;
        type Installs = Connection;

        fn install(ctx: &FeatureInitContext, params: &MachineParams) -> anyhow::Result<Connection> {
            ctx.install::<Connection>(&params.context)
        }

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            cx.outlet()
        }
    }
    //@show-end
}

mod layout_installing {
    //@show a layout that installs
    //@hide
    use guinea_core::feature::Bound;
    use guinea_core::scope::Reducer;
    use guinea_macros::{feature, installs};
    //@unhide
    use guinea_winui::{FeatureInitContext, Layout, LayoutCx, layout};
    use windows_reactor::View;
    //@hide

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Sidebar(pub bool);

    impl Reducer for Sidebar {
        type Update = bool;

        fn reduce(&mut self, open: bool) {
            self.0 = open;
        }
    }

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Metrics(pub u32);

    impl Reducer for Metrics {
        type Update = u32;

        fn reduce(&mut self, to: u32) {
            self.0 = to;
        }
    }

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Title(pub String);

    impl Reducer for Title {
        type Update = String;

        fn reduce(&mut self, to: String) {
            self.0 = to;
        }
    }

    feature! { pub SidebarFeature { exports { Sidebar } } }
    feature! { pub MetricsFeature { exports { Metrics } } }

    #[installs]
    fn sidebar(cx: &FeatureInitContext) -> anyhow::Result<SidebarFeature> {
        Ok(SidebarFeature(cx.state::<Sidebar>().plain()))
    }

    #[installs]
    fn metrics(cx: &FeatureInitContext) -> anyhow::Result<MetricsFeature> {
        Ok(MetricsFeature(cx.state::<Metrics>().plain()))
    }
    //@unhide

    /// How often metrics refresh, when the application does not say.
    #[derive(Clone, Default)]
    pub struct Refresh {
        pub every_ms: u64,
    }

    #[derive(Default)]
    pub struct Shell;

    #[layout]
    impl Layout for Shell {
        type Params = ();
        type Installs = (SidebarFeature, MetricsFeature, Bound<Title>);

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self::Installs> {
            // A setting the application may `provide`, and a default when
            // it does not.
            let refresh = ctx.require_or_default::<Refresh>();
            let title = ctx
                .state::<Title>()
                .seed(Title(format!("every {} ms", refresh.every_ms)));

            Ok((ctx.install(&())?, ctx.install(&())?, title.plain()))
        }

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            cx.outlet()
        }
    }
    //@show-end
}

mod minding_the_layout {
    //@show a layout that minds being left
    use guinea_winui::{Ask, Layout, LayoutCx, Verdict, layout};
    use windows_reactor::View;

    #[derive(Default)]
    pub struct Wizard {
        step: u32,
        finished: bool,
    }

    #[layout]
    impl Layout for Wizard {
        type Params = ();

        fn leaving(&self) -> Verdict {
            if self.step == 0 || self.finished {
                Verdict::Allow
            } else {
                Verdict::ask(Ask::new("Abandon the setup?", "Abandon", "Continue"))
            }
        }

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            cx.outlet()
        }
    }

    #[test]
    fn asks_only_halfway() {
        let mut wizard = Wizard::default();
        assert!(matches!(wizard.leaving(), Verdict::Allow));

        wizard.step = 2;
        assert!(matches!(wizard.leaving(), Verdict::Ask(..)));
    }
    //@show-end
}

mod tab_strip {
    //@show a tab strip
    use guinea_winui::{Layout, LayoutCx, Page, PageCx, layout, page};
    use windows_reactor::{ChildrenControl, StackPanel, TextBlock, View};
    //@hide

    #[derive(Default)]
    pub struct Processes;

    #[page]
    impl Page for Processes {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("processes").into()
        }
    }

    #[derive(Default)]
    pub struct Services;

    #[page]
    impl Page for Services {
        fn view(&self, _cx: &mut PageCx<'_, Self>) -> View {
            TextBlock::new().text("services").into()
        }
    }
    //@unhide

    #[derive(Default)]
    pub struct Tabs;

    #[layout]
    impl Layout for Tabs {
        type Params = ();

        fn view(&self, cx: &mut LayoutCx<'_, Self>) -> View {
            let tab = |name: &str, current: bool| {
                TextBlock::new().text(if current {
                    format!("[{name}]")
                } else {
                    name.to_string()
                })
            };

            StackPanel::new()
                .children((
                    tab("Processes", cx.child_is::<Processes>()),
                    tab("Services", cx.child_is::<Services>()),
                    cx.outlet(),
                ))
                .into()
        }
    }
    //@show-end
}
