//! What the application installs, as the pages of a route tree see it.
//!
//! A tree lists in `app { .. }` the features and plugins whose exports its
//! pages read. The list is what lets a read compile; navigating is where an
//! application that did not install something on it finds out.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use guinea_app::app::{AppFeature, FeatureBuilder, GuineaApp, Plugin, PluginBuilder};
use guinea_app::feature::FeatureInitContext;
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::Reducer;
use guinea_macros::routes;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page};
use guinea_router::router::Router;

#[derive(Clone, Debug, Default)]
struct Language(&'static str);

impl Reducer for Language {
    type Update = &'static str;

    fn reduce(&mut self, to: &'static str) {
        self.0 = to;
    }
}

#[derive(Clone, Debug, Default)]
struct Accent(&'static str);

impl Reducer for Accent {
    type Update = &'static str;

    fn reduce(&mut self, to: &'static str) {
        self.0 = to;
    }
}

struct Localisation;

impl AppFeature for Localisation {
    type Exports = (Language,);

    fn install(self, app: &mut FeatureBuilder) -> anyhow::Result<()> {
        app.state::<Language>().seed(Language("en")).plain();
        Ok(())
    }
}

struct Accents;

impl Plugin for Accents {
    const ID: &'static str = "test.accents";
    type Exports = (Accent,);

    fn build(self, app: &mut PluginBuilder) -> anyhow::Result<()> {
        app.state::<Accent>().seed(Accent("blue")).plain();
        Ok(())
    }
}

thread_local! {
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn seen(what: String) {
    SEEN.with(|seen| seen.borrow_mut().push(what));
}

struct Shell;

impl Layout for Shell {
    type Params = ShellParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ShellParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        let (language, _) = cx.read::<Language, _>();
        seen(format!("shell speaks {}", language.0));
        cx.outlet();
    }
}

struct Reader;

impl Page for Reader {
    type Params = ReaderParams;
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &ReaderParams) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        let (language, _) = cx.read::<Language, _>();
        let (accent, _) = cx.read::<Accent, _>();
        seen(format!("page speaks {} in {}", language.0, accent.0));
    }
}

routes! {
    backend = guinea_router::headless::Headless,
    Route {
        app {
            feature(Localisation)
            #[cfg(all())]
            plugin(Accents)
            #[cfg(any())]
            plugin(NotEvenBuilt)
        }
        layout(Shell) {
            page(Reader)
        }
    }
}

fn router(app: GuineaApp) -> Rc<Router<Headless>> {
    SEEN.with(|seen| seen.borrow_mut().clear());

    let token = UiThreadToken::dangerously_create_token_unchecked();
    let runtime = app.install(token.clone()).expect("install");
    guinea_app::app::install_runtime(runtime);

    Rc::new(Router::<Headless>::new(token))
}

#[test]
fn a_page_and_its_layout_read_what_the_listed_application_installs() {
    let router = router(GuineaApp::new().feature(Localisation).plugin(Accents));

    router.navigate(Route::Reader {}).expect("to the reader");
    router.render(&());

    assert_eq!(
        SEEN.with(|seen| seen.borrow().clone()),
        ["shell speaks en", "page speaks en in blue"]
    );
}

#[test]
fn navigating_without_something_the_tree_lists_panics_naming_it() {
    let router = router(GuineaApp::new().plugin(Accents));

    let outcome = catch_unwind(AssertUnwindSafe(|| router.navigate(Route::Reader {}).map(|_| ())));

    let message = match &outcome {
        Err(panic) => panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|it| it.to_string())),
        Ok(_) => None,
    };
    assert!(
        message.as_deref().is_some_and(|it| it.contains("feature(Localisation)")),
        "got {message:?}, from {:?}",
        outcome.as_ref().map(|_| ())
    );
}
