//! Frames without a window: egui draws into a context nobody shows, which is
//! all a mount, a navigation and a read need.

use std::cell::Cell;

use guinea_app::app::{GuineaHost, roots};
use guinea_app::feature::FeatureInitContext;
use guinea_core::scope::Scope;
use guinea_macros::{feature, installs, reducer};

use crate::run::{Root, frame};
use crate::{Layout, LayoutCx, Navigator, Page, PageCx, Routable, layout, page};

#[derive(Clone, PartialEq, Debug, Routable)]
enum Place {
    Counting,
    Remembering,
    Other,
    Broken,
    Reading,
    Leaving,
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Shared(u32);

#[reducer]
fn shared(this: &mut Shared, to: u32) {
    this.0 = to;
}

feature! {
    Sharing {
        exports { Shared }
    }
}

#[installs]
fn sharing(cx: &FeatureInitContext) -> anyhow::Result<Sharing> {
    Ok(Sharing(cx.state::<Shared>().seed(Shared(7)).plain()))
}

#[derive(Default)]
struct Shell;

#[layout]
impl Layout for Shell {
    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        ctx.install::<Sharing>(&())?;
        Ok(Self)
    }

    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Place>();
        let page = cx.outlet(&nav, |place, cx| match place {
            Place::Counting => cx.mount::<Counting>(&()),
            Place::Remembering => cx.mount::<Remembering>(&()).keep(),
            Place::Other => cx.mount::<Other>(&()),
            Place::Broken => cx.mount::<Broken>(&()),
            Place::Reading => cx.mount::<Reading>(&()),
            Place::Leaving => cx.mount::<Leaving>(&()),
        });
        page.draw(cx.ui());
    }
}

/// Counts the frames it was drawn in: its state is its own, from one to the
/// next.
#[derive(Default)]
struct Counting {
    frames: u32,
}

#[page]
impl Page for Counting {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        self.frames += 1;
        cx.ui().label(format!("frame {}", self.frames));
    }
}

/// [`Counting`], mounted to be kept.
#[derive(Default)]
struct Remembering {
    frames: u32,
}

#[page]
impl Page for Remembering {
    fn render(&mut self, _cx: &mut PageCx<'_, Self>) {
        self.frames += 1;
    }
}

#[derive(Default)]
struct Other;

#[page]
impl Page for Other {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        cx.ui().label("other");
    }
}

#[derive(Default)]
struct Broken;

#[page]
impl Page for Broken {
    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        anyhow::bail!("the agent is not reachable")
    }

    fn render(&mut self, _cx: &mut PageCx<'_, Self>) {}
}

thread_local! {
    static READ: Cell<Option<u32>> = const { Cell::new(None) };
    static LEAVE: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
struct Reading;

#[page]
impl Page for Reading {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (shared, _) = cx.read::<Shared>();
        READ.set(Some(shared.0));
    }
}

/// Navigates away from inside its own drawing, when told to.
#[derive(Default)]
struct Leaving;

#[page]
impl Page for Leaving {
    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        if LEAVE.take() {
            let _moved = cx.navigate::<Place>().to(Place::Other);
        }
    }
}

struct Window {
    root: roots::Root,
    nav: Navigator<Place>,
    egui: egui::Context,
    _app: GuineaHost,
}

impl Window {
    fn at(place: Place) -> Self {
        let app = GuineaHost::new();
        let root = roots::Root::under(&app.context());
        let nav = Navigator::open(root.scope(), place);
        let window = Self {
            root,
            nav,
            egui: egui::Context::default(),
            _app: app,
        };
        window.draw();
        window
    }

    fn draw(&self) {
        let root = Root::of::<Shell>();
        let mut output = self
            .egui
            .run_ui(egui::RawInput::default(), |ui| frame(self.root.scope(), &root, ui));
        output.textures_delta.clear();
    }

    fn go(&self, place: Place) {
        let _moved = self.nav.to(place);
        self.draw();
    }

    fn scope<N: 'static>(&self) -> Option<Scope> {
        self.root
            .scope()
            .subtree()
            .into_iter()
            .find(|scope| guinea_router::mount::name(*scope) == Some(std::any::type_name::<N>()))
    }

    fn awake<N: 'static>(&self) -> bool {
        self.scope::<N>().is_some_and(|scope| scope.is_awake())
    }

    fn node<N: 'static, T>(&self, read: impl FnOnce(&mut N) -> T) -> Option<T> {
        self.scope::<N>()?.kept::<N, _>(read)
    }
}

#[test]
fn what_is_mounted_says_where_it_was_written() {
    use guinea_core::__private::observer;
    use guinea_core::observability::Event;

    let window = Window::at(Place::Counting);
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
    assert_eq!(written, ["Shell", "Counting"]);
}

#[test]
fn a_page_s_state_lasts_from_frame_to_frame() {
    let window = Window::at(Place::Counting);
    window.draw();
    window.draw();

    assert_eq!(window.node::<Counting, _>(|page| page.frames), Some(3));
}

#[test]
fn a_navigation_is_drawn_in_the_next_frame_and_what_was_left_goes() {
    let window = Window::at(Place::Counting);

    window.go(Place::Other);

    assert!(window.awake::<Other>());
    assert!(window.scope::<Counting>().is_none(), "a page not kept goes when it is left");
}

#[test]
fn a_kept_page_comes_back_as_it_was_left() {
    let window = Window::at(Place::Remembering);
    window.draw();

    window.go(Place::Other);
    assert!(!window.awake::<Remembering>(), "asleep while left");
    window.go(Place::Remembering);

    assert_eq!(window.node::<Remembering, _>(|page| page.frames), Some(3));
}

#[test]
fn a_page_not_kept_starts_again() {
    let window = Window::at(Place::Counting);
    window.draw();

    window.go(Place::Other);
    window.go(Place::Counting);

    assert_eq!(window.node::<Counting, _>(|page| page.frames), Some(1));
}

#[test]
fn a_page_reads_what_the_layout_above_exports() {
    READ.set(None);

    let _window = Window::at(Place::Reading);

    assert_eq!(READ.get(), Some(7));
}

#[test]
fn a_place_that_fails_to_mount_leaves_the_window_where_it_was() {
    let window = Window::at(Place::Other);

    window.go(Place::Broken);

    assert_eq!(window.nav.current(), Some(Place::Other));
    assert_eq!(window.nav.failure().as_deref(), Some("Broken did not mount: the agent is not reachable"));
    assert!(window.awake::<Other>());
}

#[test]
fn a_navigation_from_inside_a_frame_is_drawn_in_the_next_one() {
    let window = Window::at(Place::Leaving);

    LEAVE.set(true);
    window.draw();
    assert!(window.awake::<Leaving>(), "the frame that navigated finishes with what it drew");

    window.draw();
    assert!(window.awake::<Other>());
    assert!(window.scope::<Leaving>().is_none());
}
