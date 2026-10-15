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
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("guinea").into()
        }
    }
    //@show-end
}

mod whole {
    //@show a whole page
    //@hide
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
    use windows_reactor::{Button, StackPanel, TextBlock, View};

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

        cx.answers(move |Add(by): Add| count.push(by));

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

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Self, Counter)> {
            Ok((Self::default(), ctx.install(&())?))
        }

        fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Bigger => self.step += 1,
            }
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (count, dispatch) = cx.read::<Count>();
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
                guinea_winui::harness::Mounted::<CounterPage>::mount(h.segment(), ()).unwrap();

            page.click_text("Bigger").settle();
            page.click_text("Add").settle();
            page.settle();

            assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
        });
    }
    //@show-end
}

mod installing {
    //@show a page that installs
    //@hide
    use guinea_core::feature::Bound;
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{FeatureInitContext, Install, Page, PageCx, page};
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
    fn processes(cx: &Install<str>) -> anyhow::Result<Processes> {
        let listing = cx.state::<Listing>().plain();
        listing.push(format!("processes on {}", cx.params()));
        Ok(Processes(listing))
    }

    #[derive(Clone, PartialEq)]
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
        ) -> anyhow::Result<(Self, Self::Installs)> {
            let processes = ctx.install::<Processes>(&params.context)?;
            let selection = ctx.state::<Selection>().plain();
            Ok((Self, (processes, selection)))
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (listing, _) = cx.read::<Listing>();
            TextBlock::new().text(listing.0.clone()).into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn lists_the_machine_it_was_reached_with() {
        guinea_app::app::check(2, |h| {
            let params = ProcessesParams {
                context: "ubuntu".into(),
            };
            let mut page =
                guinea_winui::harness::Mounted::<ProcessesPage>::mount(h.segment(), params)
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
    use guinea_winui::{FeatureInitContext, Page, PageCx, page};
    use windows_reactor::{TextBlock, View};

    #[derive(Clone, PartialEq)]
    pub struct ProcessParams {
        pub pid: u32,
    }

    pub struct ProcessPage {
        pid: u32,
    }

    #[page]
    impl Page for ProcessPage {
        type Params = ProcessParams;

        fn install(_ctx: &FeatureInitContext, params: &ProcessParams) -> anyhow::Result<(Self, ())> {
            Ok((Self { pid: params.pid }, ()))
        }

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new()
                .text(format!("process {}", self.pid))
                .into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn shows_the_pid_it_was_reached_with() {
        guinea_app::app::check(2, |h| {
            let page = guinea_winui::harness::Mounted::<ProcessPage>::mount(
                h.segment(),
                ProcessParams { pid: 42 },
            )
            .unwrap();
            assert!(page.find_text("process 42").is_some(), "{:#?}", page.tree());
        });
    }
    //@unhide
    //@show-end
}

mod starting_from_a_setting {
    //@show a page that starts from a setting
    use guinea_winui::{FeatureInitContext, Page, PageCx, UpdateCx, page};
    use windows_reactor::{Button, StackPanel, TextBlock, View};

    /// Column widths as the application saved them; a default when it did not.
    #[derive(Clone)]
    pub struct SavedWidths(pub Vec<u32>);

    impl Default for SavedWidths {
        fn default() -> Self {
            Self(vec![280, 80])
        }
    }

    /// The widths are the page's own once it is mounted: a drag changes them
    /// here, and nothing else needs to know. No `Default`: `install` is the
    /// one place a table is made.
    pub struct Table {
        widths: Vec<u32>,
    }

    pub enum Msg {
        Widen,
    }

    #[page]
    impl Page for Table {
        type Message = Msg;

        fn install(
            _ctx: &FeatureInitContext,
            _params: &(),
            saved: Option<SavedWidths>,
        ) -> anyhow::Result<(Self, ())> {
            Ok((Self { widths: saved.unwrap_or_default().0 }, ()))
        }

        fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Widen => self.widths[0] += 20,
            }
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            StackPanel::new()
                .children((
                    TextBlock::new().text(format!("widths {:?}", self.widths)),
                    Button::new()
                        .on_click(cx.on(|()| Msg::Widen))
                        .content(TextBlock::new().text("Widen")),
                ))
                .into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn starts_from_what_the_application_saved_and_changes_it_itself() {
        guinea_app::app::check(2, |h| {
            h.provide(SavedWidths(vec![300, 60]));
            let mut page = guinea_winui::harness::Mounted::<Table>::mount(h.segment(), ()).unwrap();
            assert!(page.find_text("widths [300, 60]").is_some(), "{:#?}", page.tree());

            page.click_text("Widen").settle();
            page.settle();
            assert!(page.find_text("widths [320, 60]").is_some(), "{:#?}", page.tree());
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

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
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

        cx.answers(move |Search(query): Search| results.push(format!("found {query}")));

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

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Self, Searching)> {
            Ok((Self::default(), ctx.install(&())?))
        }

        fn update(&mut self, message: Msg, cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Typed(text) => self.query = text,
                Msg::Submitted => {
                    let (_, search) = cx.read::<Results>();
                    search.emit(Search(self.query.clone()));
                }
            }
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (results, _) = cx.read::<Results>();
            TextBlock::new().text(results.0.clone()).into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn submitting_asks_for_what_was_typed() {
        guinea_app::app::check(2, |h| {
            let mut page =
                guinea_winui::harness::Mounted::<SearchPage>::mount(h.segment(), ()).unwrap();
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
    use std::rc::Rc;

    use guinea_winui::{Page, PageCx, UpdateCx, page};
    use windows_reactor::{StackPanel, TextBlock, TextBox, View};

    #[derive(Default)]
    pub struct Greeting {
        name: Rc<str>,
    }

    pub enum Msg {
        Named(Rc<str>),
    }

    #[page]
    impl Page for Greeting {
        type Message = Msg;

        fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
            match message {
                Msg::Named(name) => self.name = name,
            }
        }

        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            StackPanel::new()
                .children((
                    TextBox::new(&self.name).on_text_changed(cx.on(Msg::Named)),
                    TextBlock::new().text(format!("hello, {}", self.name)),
                ))
                .into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn greets_whoever_was_typed() {
        guinea_app::app::check(2, |h| {
            let mut page =
                guinea_winui::harness::Mounted::<Greeting>::mount(h.segment(), ()).unwrap();
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
    use guinea_core::feature::Bound;
    use guinea_macros::{feature, installs, reducer};
    //@unhide
    use guinea_winui::{
        FeatureInitContext, Layout, LayoutCx, Page, PageCx, Routable, UpdateCx, layout, page,
    };
    use windows_reactor::{Border, Button, StackPanel, TextBlock, View};

    /// Where the shell can be.
    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum Place {
        Home,
        About,
    }

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

        cx.answers(move |SetOpen(open): SetOpen| sidebar.push(open));

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
        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            // Installed by the shell above, and readable here because `Chrome`
            // exports it. A page outside the shell asking for it panics on
            // its first render, naming what it was mounted under.
            let (sidebar, _) = cx.read::<Sidebar>();
            let width = if sidebar.open { "narrow" } else { "wide" };

            TextBlock::new().text(format!("home, {width}")).into()
        }
    }

    #[derive(Default)]
    pub struct About;

    #[page]
    impl Page for About {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("guinea").into()
        }
    }

    #[derive(Default)]
    pub struct Shell;

    pub enum ShellMsg {
        Toggle,
    }

    #[layout]
    impl Layout for Shell {
        /// A flat list: a feature, and a reducer claimed directly. Pages below
        /// may read `Sidebar`, which `Chrome` exports; `Title` is the shell's
        /// own.
        type Installs = (Chrome, Bound<Title>);
        type Message = ShellMsg;

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Self, Self::Installs)> {
            let title = ctx.state::<Title>().seed(Title("guinea".into())).plain();
            Ok((Self, (ctx.install(&())?, title)))
        }

        fn update(&mut self, message: ShellMsg, cx: &mut UpdateCx<'_, Self>) {
            match message {
                ShellMsg::Toggle => {
                    let (sidebar, dispatch) = cx.read::<Sidebar>();
                    dispatch.emit(SetOpen(!sidebar.open));
                }
            }
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let (sidebar, _) = cx.read::<Sidebar>();
            let (title, _) = cx.read::<Title>();
            let nav = cx.navigate::<Place>();

            let tab = match nav.current() {
                Some(Place::Home) => "> Home",
                _ => "Home",
            };
            let side = match sidebar.open {
                true => Border::new().content(TextBlock::new().text(tab)),
                false => Border::new(),
            };

            StackPanel::new()
                .children((
                    TextBlock::new().text(title.0.clone()),
                    Button::new()
                        .on_click(cx.on(|()| ShellMsg::Toggle))
                        .content(TextBlock::new().text("Menu")),
                    side,
                    cx.outlet(&nav, |place, cx| match place {
                        Place::Home => cx.mount::<Home>(&()),
                        Place::About => cx.mount::<About>(&()),
                    }),
                ))
                .into()
        }
    }
    //@show-end

    #[cfg(feature = "harness")]
    #[test]
    fn the_shell_marks_the_tab_and_closes_its_sidebar() {
        use guinea_winui::harness::Mounted;

        guinea_app::app::check(4, |h| {
            let mut shell = Mounted::<Shell>::mount_at(h.segment(), (), Place::Home).unwrap();

            assert!(shell.find_text("guinea").is_some(), "{:#?}", shell.tree());
            assert!(shell.find_text("> Home").is_some(), "{:#?}", shell.tree());

            shell.click_text("Menu").settle();
            shell.settle();

            assert!(shell.find_text("> Home").is_none(), "{:#?}", shell.tree());
            assert!(!h.state::<Sidebar>().open);
        });
    }

    #[cfg(feature = "harness")]
    #[test]
    fn the_page_it_mounts_reads_what_it_exports() {
        use guinea_winui::harness::Mounted;

        guinea_app::app::check(4, |h| {
            let mut shell = Mounted::<Shell>::mount_at(h.segment(), (), Place::Home).unwrap();
            assert!(shell.find_text("home, narrow").is_some(), "{:#?}", shell.tree());

            shell.click_text("Menu").settle();
            shell.settle();

            assert!(shell.find_text("home, wide").is_some(), "{:#?}", shell.tree());
        });
    }
}

mod per_machine {
    //@show a layout per machine
    //@hide
    use guinea_macros::{feature, installs, reducer};
    use guinea_winui::{Install, Page, PageCx, page};
    use windows_reactor::TextBlock;
    //@unhide
    use guinea_winui::{FeatureInitContext, Layout, LayoutCx, layout};
    use windows_reactor::View;
    //@hide

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Machine(pub String);

    #[reducer]
    fn machine(this: &mut Machine, to: String) {
        this.0 = to;
    }

    feature! {
        pub Connection {
            exports { Machine }
        }
    }

    #[installs]
    fn connection(cx: &Install<str>) -> anyhow::Result<Connection> {
        let machine = cx.state::<Machine>().plain();
        machine.push(cx.params().to_string());
        Ok(Connection(machine))
    }

    #[derive(Default)]
    pub struct Overview;

    #[page]
    impl Page for Overview {
        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (machine, _) = cx.read::<Machine>();
            TextBlock::new().text(format!("on {}", machine.0)).into()
        }
    }
    //@unhide

    #[derive(Default)]
    pub struct MachineLayout;

    #[layout]
    impl Layout for MachineLayout {
        /// Which machine: `cx.mount::<MachineLayout>(&machine)`.
        type Params = String;
        type Installs = Connection;

        fn install(
            ctx: &FeatureInitContext,
            machine: &String,
        ) -> anyhow::Result<(Self, Connection)> {
            Ok((Self, ctx.install::<Connection>(machine)?))
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            cx.mount::<Overview>(&()).into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn what_it_mounts_reads_the_machine_it_was_mounted_for() {
        guinea_app::app::check(2, |h| {
            let layout = guinea_winui::harness::Mounted::<MachineLayout>::mount(
                h.segment(),
                "ubuntu".to_string(),
            )
            .unwrap();
            assert!(layout.find_text("on ubuntu").is_some(), "{:#?}", layout.tree());
        });
    }
    //@unhide
    //@show-end
}

mod layout_installing {
    //@show a layout that installs
    //@hide
    use guinea_core::feature::Bound;
    use guinea_macros::{feature, installs, reducer};
    use guinea_winui::{Page, PageCx, page};
    use windows_reactor::TextBlock;
    //@unhide
    use guinea_winui::{FeatureInitContext, Layout, LayoutCx, layout};
    use windows_reactor::View;
    //@hide

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Sidebar(pub bool);

    #[reducer]
    fn reduce_sidebar(this: &mut Sidebar, open: bool) {
        this.0 = open;
    }

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Metrics(pub u32);

    #[reducer]
    fn reduce_metrics(this: &mut Metrics, to: u32) {
        this.0 = to;
    }

    #[derive(Default, Clone, PartialEq, Debug)]
    pub struct Title(pub String);

    #[reducer]
    fn title(this: &mut Title, to: String) {
        this.0 = to;
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

    #[derive(Default)]
    pub struct MetricsPage;

    #[page]
    impl Page for MetricsPage {
        fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
            let (metrics, _) = cx.read::<Metrics>();
            TextBlock::new().text(format!("metrics {}", metrics.0)).into()
        }
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
        type Installs = (SidebarFeature, MetricsFeature, Bound<Title>);

        fn install(
            ctx: &FeatureInitContext,
            _params: &(),
            refresh: Option<Refresh>,
        ) -> anyhow::Result<(Self, Self::Installs)> {
            // A setting the application may `provide`, and a default when
            // it does not.
            let refresh = refresh.unwrap_or_default();
            let title = ctx
                .state::<Title>()
                .seed(Title(format!("every {} ms", refresh.every_ms)));

            Ok((Self, (ctx.install(&())?, ctx.install(&())?, title.plain())))
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            cx.mount::<MetricsPage>(&()).into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn starts_from_the_setting_the_application_provided() {
        guinea_app::app::check(2, |h| {
            h.provide(Refresh { every_ms: 500 });
            let shell = guinea_winui::harness::Mounted::<Shell>::mount(h.segment(), ()).unwrap();
            assert!(shell.find_text("metrics 0").is_some(), "{:#?}", shell.tree());
            assert_eq!(h.state::<Title>().0, "every 500 ms");
        });
    }
    //@unhide
    //@show-end
}

mod minding_the_layout {
    //@show a layout that minds being left
    //@hide
    use guinea_winui::{Page, PageCx, Routable, page};
    use windows_reactor::TextBlock;
    //@unhide
    use guinea_winui::{Ask, Layout, LayoutCx, Verdict, layout};
    use windows_reactor::View;
    //@hide

    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum Step {
        Name,
        Disk,
    }

    #[derive(Default)]
    pub struct StepPage;

    #[page]
    impl Page for StepPage {
        type Params = Step;

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("step").into()
        }
    }
    //@unhide

    #[derive(Default)]
    pub struct Wizard {
        step: u32,
        finished: bool,
    }

    #[layout]
    impl Layout for Wizard {
        fn leaving(&self) -> Verdict {
            if self.step == 0 || self.finished {
                Verdict::Allow
            } else {
                Verdict::ask(Ask::new("Abandon the setup?", "Abandon", "Continue"))
            }
        }

        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<Step>();
            cx.outlet(&nav, |step, cx| cx.mount::<StepPage>(step))
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
    //@hide
    use guinea_winui::{Page, PageCx, page};
    //@unhide
    use guinea_winui::{Layout, LayoutCx, Routable, layout};
    use windows_reactor::{StackPanel, TextBlock, View};

    #[derive(Clone, PartialEq, Debug, Routable)]
    pub enum Tab {
        Processes,
        Services,
    }
    //@hide

    #[derive(Default)]
    pub struct Processes;

    #[page]
    impl Page for Processes {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("processes").into()
        }
    }

    #[derive(Default)]
    pub struct Services;

    #[page]
    impl Page for Services {
        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> View {
            TextBlock::new().text("services").into()
        }
    }
    //@unhide

    #[derive(Default)]
    pub struct Tabs;

    #[layout]
    impl Layout for Tabs {
        fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
            let nav = cx.navigate::<Tab>();
            let here = nav.current();
            let tab = |name: &str, tab: Tab| {
                TextBlock::new().text(if here == Some(tab) {
                    format!("[{name}]")
                } else {
                    name.to_string()
                })
            };

            StackPanel::new()
                .children((
                    tab("Processes", Tab::Processes),
                    tab("Services", Tab::Services),
                    cx.outlet(&nav, |tab, cx| match tab {
                        Tab::Processes => cx.mount::<Processes>(&()),
                        Tab::Services => cx.mount::<Services>(&()),
                    }),
                ))
                .into()
        }
    }
    //@hide

    #[cfg(feature = "harness")]
    #[test]
    fn marks_the_tab_the_navigator_is_at() {
        guinea_app::app::check(2, |h| {
            let mut tabs =
                guinea_winui::harness::Mounted::<Tabs>::mount_at(h.segment(), (), Tab::Services).unwrap();
            assert!(tabs.find_text("[Services]").is_some(), "{:#?}", tabs.tree());
            assert!(tabs.find_text("services").is_some(), "{:#?}", tabs.tree());

            tabs.navigator::<Tab>().to(Tab::Processes);
            tabs.settle();
            assert!(tabs.find_text("[Processes]").is_some(), "{:#?}", tabs.tree());
        });
    }
    //@unhide
    //@show-end
}
