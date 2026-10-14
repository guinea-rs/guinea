#![cfg(all(windows, feature = "winui"))]

//! Mounting driven through the facade, the way an application sees it.

mod provided {
    use guinea::feature::{FeatureHost, FeatureInitContext};
    use guinea::mount::Pass;
    use guinea::winui::*;

    #[derive(Clone)]
    struct Greeting(&'static str);

    struct GreetingPlugin;

    impl guinea_app::app::Plugin for GreetingPlugin {
        const ID: &'static str = "test.greeting";
        type Exports = ();

        fn build(self, app: &mut guinea_app::app::PluginBuilder) -> anyhow::Result<()> {
            app.provide(Greeting("hello"));
            Ok(())
        }
    }

    guinea_macros::feature! {
        Greeted {}
    }

    #[guinea_macros::installs]
    fn greeted(_cx: &FeatureInitContext, greeting: Greeting) -> anyhow::Result<Greeted> {
        assert_eq!(greeting.0, "hello");
        Ok(Greeted)
    }

    #[derive(Default)]
    struct NeedsAService;

    #[page]
    impl Page for NeedsAService {
        type Installs = Greeted;

        fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<(Self, Greeted)> {
            Ok((Self::default(), ctx.install(&())?))
        }

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> windows_reactor::View {
            windows_reactor::Grid::new().into()
        }
    }

    fn mount(host: &FeatureHost) -> anyhow::Result<()> {
        let pass = Pass::begin(host.scope());
        let mounted = pass.mount::<NeedsAService, ()>(&(), |cx| <NeedsAService as Mountable>::install(cx, &()));
        pass.end();
        mounted.map(|_| ())
    }

    #[test]
    fn a_page_s_feature_takes_what_an_application_plugin_provided() {
        let runtime = guinea::app::GuineaApp::new()
            .plugin(GreetingPlugin)
            .install()
            .expect("install");

        mount(&FeatureHost::under(&runtime.context()))
            .expect("the page's feature must be handed the service the plugin provided");
    }

    #[test]
    fn a_page_without_an_application_gets_a_plain_error_not_a_panic() {
        let err = mount(&FeatureHost::detached()).expect_err("nothing provided the service");

        assert!(
            format!("{err:#}").contains("Greeting"),
            "the error should name the missing service, got: {err:#}"
        );
    }
}

mod a_reducer_names_only_what_it_writes {
    use guinea_core::scope::ScopeTree;
    use guinea_macros::reducer;

    /// The whole declaration is here, and the central type of the feature -
    /// the one every other file names - is the one written, not one derived
    /// from the function's name by changing its case.
    #[derive(Clone, Default, Debug)]
    struct Widget {
        value: i32,
    }

    #[reducer]
    fn widget(this: &mut Widget, to: i32) {
        this.value = to;
    }

    #[test]
    fn the_state_is_the_reducer() {
        let scope = ScopeTree::new();
        scope.push::<Widget>(42);

        assert_eq!(scope.state::<Widget>().borrow().value, 42);
    }
}
