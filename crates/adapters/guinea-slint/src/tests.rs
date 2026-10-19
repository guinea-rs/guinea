//! A window without a loop: Slint's testing backend makes the component, and
//! the navigator is moved from here or through the component's global, the
//! way the markup moves it.

use guinea_app::app::{GuineaHost, roots};
use guinea_app::feature::FeatureInitContext;
use guinea_core::guard::Verdict;
use guinea_core::scope::Scope;
use guinea_macros::{feature, installs, reducer};
use slint::ComponentHandle;

use crate::run::Opened;
use crate::{Ask, BindCx, Layout, LayoutCx, Page, Routable, Shows, Window, layout, page};

slint::slint! {
    export enum RouteId { processes, remembering, other, broken, draft, nested }

    export global Navigator {
        in property <RouteId> route;
        in property <string> failure;
        in property <bool> asking;
        in property <string> question;
        callback go-other();
        callback answer(bool);
    }

    export global Model {
        in property <string> label;
    }

    export component Host inherits Window {
        Text {
            text: Model.label + Navigator.question + Navigator.failure;
        }
    }
}

#[derive(Clone, PartialEq, Debug, Routable)]
enum Place {
    Processes,
    Remembering,
    Other,
    Broken,
    Draft,
    Nested,
}

/// What `guinea-slint-build` writes from `Place`, cut down to what these
/// tests look at.
impl Shows<Host> for Place {
    fn show(window: &Host, nav: &crate::Navigator<Self>) {
        let global = window.global::<Navigator>();
        let ask = nav.pending();

        global.set_failure(nav.failure().unwrap_or_default().into());
        global.set_asking(ask.is_some());
        global.set_question(ask.map(|ask| ask.text).unwrap_or_default().into());

        let Some(place) = nav.current() else {
            return;
        };
        global.set_route(match place {
            Place::Processes => RouteId::Processes,
            Place::Remembering => RouteId::Remembering,
            Place::Other => RouteId::Other,
            Place::Broken => RouteId::Broken,
            Place::Draft => RouteId::Draft,
            Place::Nested => RouteId::Nested,
        });
    }

    fn listen(window: &Host, nav: &crate::Navigator<Self>) {
        let global = window.global::<Navigator>();
        global.on_go_other({
            let nav = nav.clone();
            move || {
                let _moved = nav.to(Place::Other);
            }
        });
        global.on_answer({
            let nav = nav.clone();
            move |allowed| nav.answer(allowed)
        });
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Label(String);

#[reducer]
fn label(this: &mut Label, to: String) {
    this.0 = to;
}

feature! {
    Labelling {
        exports { Label }
    }
}

#[installs]
fn labelling(cx: &FeatureInitContext) -> anyhow::Result<Labelling> {
    Ok(Labelling(cx.state::<Label>().seed(Label("processes".into())).plain()))
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn mount(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Place>();
        cx.outlet(&nav, |place, cx| match place {
            Place::Processes => cx.mount::<Processes>(&()),
            Place::Remembering => cx.mount::<Other>(&"remembering").keep(),
            Place::Other => cx.mount::<Other>(&"other"),
            Place::Broken => cx.mount::<Broken>(&()),
            Place::Draft => cx.mount::<Draft>(&()),
            Place::Nested => cx.mount::<Area>(&()),
        });
    }
}

/// Binds its label to the global the markup reads.
#[derive(Default)]
struct Processes;

#[page]
impl Page for Processes {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Labelling>(&())?;
        Ok(Self)
    }

    fn bind(&mut self, cx: &BindCx<Self>) {
        let root = cx.root::<Host>();
        cx.bind::<Label>(move |label| root.global::<Model>().set_label(label.0.as_str().into()));
    }
}

#[derive(Default)]
struct Other;

#[page]
impl Page for Other {
    type Params = &'static str;
}

#[derive(Default)]
struct Broken;

#[page]
impl Page for Broken {
    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        anyhow::bail!("the agent is not reachable")
    }
}

/// Minds being left: every attempt to go puts a question.
#[derive(Default)]
struct Draft;

#[page]
impl Page for Draft {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.on_leave(|| Verdict::ask(Ask::new("Discard the draft?", "Discard", "Keep editing")));
        Ok(Self)
    }
}

/// A layout under the shell, with a page of its own under it.
#[derive(Default)]
struct Area;

#[layout]
impl Layout for Area {
    fn mount(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let _inner = cx.mount::<Other>(&"inner");
    }
}

#[derive(Default)]
struct Unreachable;

#[layout]
impl Layout for Unreachable {
    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        anyhow::bail!("the agent is not reachable")
    }

    fn mount(&mut self, _cx: &mut LayoutCx<'_, Self>) {}
}

struct Open {
    root: roots::Root,
    component: Host,
    nav: crate::Navigator<Place>,
    _opened: Opened,
    _app: GuineaHost,
}

impl Open {
    fn at(place: Place) -> Self {
        i_slint_backend_testing::init_no_event_loop();

        let app = GuineaHost::new();
        let root = roots::Root::under(&app.context());
        let component = Host::new().expect("the component");
        let opened = Window::of::<Shell>(component.clone_strong())
            .at(place)
            .open(root.scope())
            .expect("the window opens");
        let nav = crate::Navigator::<Place>::find(root.scope()).expect("the window's navigator");

        Self {
            root,
            component,
            nav,
            _opened: opened,
            _app: app,
        }
    }

    fn scopes(&self, named: &str) -> Vec<Scope> {
        self.root
            .scope()
            .subtree()
            .into_iter()
            .filter(|scope| guinea_router::mount::name(*scope).is_some_and(|name| name.ends_with(named)))
            .collect()
    }

    fn one(&self, named: &str) -> Option<Scope> {
        self.scopes(named).first().copied()
    }

    fn global(&self) -> Navigator<'_> {
        self.component.global::<Navigator>()
    }
}

#[test]
fn what_is_mounted_says_where_it_was_written() {
    use guinea_core::__private::observer;
    use guinea_core::observability::Event;

    let window = Open::at(Place::Processes);
    let watching = observer::watch(window.root.scope(), || {}).expect("nothing else watches");

    let events = watching.take().events;

    let written: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            Event::Mounted { name, written: Some(at), .. } if at.file.ends_with("tests.rs") => {
                Some(guinea_router::observability::short(name))
            }
            _ => None,
        })
        .collect();
    assert_eq!(written, ["Shell", "Processes"]);
}

#[test]
fn the_page_the_navigator_is_at_is_mounted_and_bound() {
    let window = Open::at(Place::Processes);

    assert!(window.one("::Processes").is_some_and(|scope| scope.is_awake()));
    assert_eq!(window.component.global::<Model>().get_label(), "processes");
}

#[test]
fn state_reaches_the_global_after_the_page_was_bound() {
    let window = Open::at(Place::Processes);
    let page = window.one("::Processes");
    assert!(page.is_some(), "mounted");

    page.into_iter().for_each(|page| page.push::<Label>("fedora".into()));

    assert_eq!(window.component.global::<Model>().get_label(), "fedora");
}

#[test]
fn the_global_shows_where_the_navigator_is() {
    let window = Open::at(Place::Processes);
    assert_eq!(window.global().get_route(), RouteId::Processes);

    let _moved = window.nav.to(Place::Draft);

    assert_eq!(window.global().get_route(), RouteId::Draft);
}

#[test]
fn a_move_mounts_the_new_page_and_lets_the_old_go() {
    let window = Open::at(Place::Processes);

    let _moved = window.nav.to(Place::Other);

    assert!(window.one("::Processes").is_none(), "a page not kept goes when it is left");
    assert!(window.one("::Other").is_some());
}

#[test]
fn a_kept_page_sleeps_while_left_and_comes_back_the_same() {
    let window = Open::at(Place::Remembering);
    let kept = window.one("::Other");
    assert!(kept.is_some(), "mounted");

    let _moved = window.nav.to(Place::Processes);
    assert!(kept.is_some_and(|kept| kept.is_alive() && !kept.is_awake()), "asleep while left");

    let _moved = window.nav.to(Place::Remembering);
    assert_eq!(window.one("::Other"), kept);
    assert!(kept.is_some_and(|kept| kept.is_awake()));
}

#[test]
fn a_place_that_fails_to_mount_leaves_the_window_where_it_was_and_says_why() {
    let window = Open::at(Place::Other);

    let _moved = window.nav.to(Place::Broken);

    assert_eq!(window.nav.current(), Some(Place::Other));
    assert_eq!(window.global().get_route(), RouteId::Other);
    assert_eq!(
        window.global().get_failure(),
        "Broken did not mount: the agent is not reachable"
    );
}

#[test]
fn a_callback_on_the_global_moves_the_window() {
    let window = Open::at(Place::Processes);

    window.global().invoke_go_other();

    assert_eq!(window.nav.current(), Some(Place::Other));
    assert!(window.one("::Other").is_some());
    assert_eq!(window.global().get_route(), RouteId::Other);
}

#[test]
fn a_question_is_shown_and_answered_through_the_global() {
    let window = Open::at(Place::Draft);

    window.global().invoke_go_other();
    assert!(window.global().get_asking());
    assert_eq!(window.global().get_question(), "Discard the draft?");
    assert!(window.one("::Draft").is_some(), "nothing moved before the answer");

    window.global().invoke_answer(true);

    assert!(!window.global().get_asking());
    assert!(window.one("::Draft").is_none());
    assert!(window.one("::Other").is_some());
}

#[test]
fn a_layout_mounts_what_is_under_it_too() {
    let window = Open::at(Place::Nested);

    assert!(window.one("::Area").is_some());
    assert!(window.one("::Other").is_some(), "the area's own page");
}

#[test]
fn a_root_that_does_not_mount_does_not_open() {
    i_slint_backend_testing::init_no_event_loop();
    let app = GuineaHost::new();
    let root = roots::Root::under(&app.context());

    let opened = Window::of::<Unreachable>(Host::new().expect("the component")).open(root.scope());

    let error = opened.err().map(|error| format!("{error:#}"));
    assert_eq!(error.as_deref(), Some("Unreachable did not mount: the agent is not reachable"));
}
