#![cfg(all(windows, feature = "winui"))]

//! Mounting driven through the facade, the way an application sees it.

mod provided {
    use guinea::app::GuineaHost;
    use guinea::app::roots::Root;
    use guinea::mount::Pass;
    use guinea::winui::*;

    #[derive(Clone)]
    struct Greeting(&'static str);

    guinea_macros::plugin! {
        Greets {
            provides { Greeting }
        }
    }

    #[guinea_macros::builds]
    fn greets(_app: &mut guinea_app::app::PluginBuilder) -> anyhow::Result<Greets> {
        Ok(Greets(Greeting("hello")))
    }

    guinea_macros::feature! {
        Greeted {}
    }

    #[guinea_macros::installs]
    fn greeted(_cx: &guinea::feature::Install, greeting: Greeting) -> anyhow::Result<Greeted> {
        assert_eq!(greeting.0, "hello");
        Ok(Greeted)
    }

    #[derive(Default)]
    struct NeedsAService;

    #[page]
    impl Page for NeedsAService {
        fn install(ctx: &Install) -> anyhow::Result<Self> {
            ctx.install::<Greeted>(&())?;
            Ok(Self::default())
        }

        fn view(&self, _cx: &mut PageCx<'_, '_, Self>) -> windows_reactor::View {
            windows_reactor::Grid::new().into()
        }
    }

    fn mount(root: &Root) -> anyhow::Result<()> {
        let pass = Pass::begin(root.scope());
        let mounted = pass.mount::<NeedsAService, ()>(&(), |cx| <NeedsAService as Mountable>::install(cx, &()));
        pass.end();
        mounted.map(|_| ())
    }

    #[test]
    fn a_page_s_feature_takes_what_the_application_provided_from_a_plugin() {
        let (host, ()) = guinea::app::start("test", |app| {
            app.plugin::<Greets>(())?;
            Ok(())
        })
        .expect("install");

        mount(&Root::under(&host.context()))
            .expect("the page's feature must be handed the service the plugin provided");
    }

    #[test]
    fn a_page_with_an_unavailable_declared_dependency_panics() {
        let host = GuineaHost::new();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mount(&Root::under(&host.context()))))
            .err().expect("missing dependencies are composition errors");
        let message = panic.downcast_ref::<String>().unwrap();
        insta::assert_snapshot!(message.replace('\\', "/"));
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
