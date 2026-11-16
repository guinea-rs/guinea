use super::*;
use crate::{Layout, LayoutCx, Page, PageCx, UpdateCx, layout, page};
use windows_reactor::{Grid, TextBlock};

#[derive(Default, Clone)]
struct Stats {
    setup_runs: usize,
    setup_active: usize,
    setup_dropped: usize,
    page_builds: usize,
    layout_builds: usize,
    feedback_installs: usize,
    feedback_dropped: usize,
}

thread_local! { static STATS: RefCell<Stats> = RefCell::default(); }
fn stats() -> Stats {
    STATS.with_borrow(Clone::clone)
}
fn reset() {
    STATS.with_borrow_mut(|s| *s = Stats::default());
}

struct Setup;
impl Setup {
    fn new() -> Self {
        STATS.with_borrow_mut(|s| s.setup_active += 1);
        Self
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        STATS.with_borrow_mut(|s| {
            s.setup_active -= 1;
            s.setup_dropped += 1;
        });
    }
}

struct GatedPage {
    value: u32,
    enabled: bool,
}
enum Change {
    Value(u32),
    Gate(bool),
}

#[page]
impl Page for GatedPage {
    type Message = Change;
    fn install(_: &crate::Install) -> anyhow::Result<Self> {
        Ok(Self {
            value: 0,
            enabled: true,
        })
    }
    fn update(&mut self, message: Change, _: &mut UpdateCx<'_, Self>) {
        match message {
            Change::Value(value) => self.value = value,
            Change::Gate(enabled) => self.enabled = enabled,
        }
    }
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        STATS.with_borrow_mut(|s| s.setup_runs += 1);
        cx.use_effect_guard("persistent setup", (), Setup::new);
        if !self.enabled {
            return TextBlock::new().text("ungated").into();
        }
        let original = std::ptr::from_ref(cx);
        cx.when_presented(|cx| {
            assert!(std::ptr::eq(original, std::ptr::from_ref(cx)));
            STATS.with_borrow_mut(|s| s.page_builds += 1);
            TextBlock::new()
                .text(format!("value {}", self.value))
                .into()
        })
    }
}

struct Feedback;
impl Drop for Feedback {
    fn drop(&mut self) {
        STATS.with_borrow_mut(|s| s.feedback_dropped += 1);
    }
}
#[page]
impl Page for Feedback {
    fn install(_: &crate::Install) -> anyhow::Result<Self> {
        STATS.with_borrow_mut(|s| s.feedback_installs += 1);
        Ok(Self)
    }
    fn view(&self, _: &mut PageCx<'_, '_, Self>) -> View {
        TextBlock::new().text("feedback").into()
    }
}

#[derive(Default)]
struct GatedLayout(u32);
#[layout]
impl Layout for GatedLayout {
    type Message = u32;
    fn update(&mut self, value: u32, _: &mut UpdateCx<'_, Self>) {
        self.0 = value;
    }
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        STATS.with_borrow_mut(|s| s.setup_runs += 1);
        cx.use_effect_guard("persistent setup", (), Setup::new);
        let feedback: View = cx.mount::<Feedback>(&()).into();
        let original = std::ptr::from_ref(cx);
        cx.when_presented(|cx| {
            assert!(std::ptr::eq(original, std::ptr::from_ref(cx)));
            STATS.with_borrow_mut(|s| s.layout_builds += 1);
            // A second gate shares the same presentation observation.
            let body =
                cx.when_presented(|_| TextBlock::new().text(format!("layout {}", self.0)).into());
            Grid::new().children((body, feedback)).into()
        })
    }
}

#[test]
fn page_gate_removes_native_content_but_keeps_setup_and_latest_page_state() {
    guinea_app::app::check(4, |h| {
        reset();
        let mut page = Mounted::<GatedPage>::mount(h.child(), ()).unwrap();
        let native = page.find_text("value 0").unwrap();
        assert_eq!((stats().page_builds, stats().setup_active), (1, 1));
        page.set_presented(false);
        assert!(page.find_text("value 0").is_none());
        assert!(page.host.runtime().graph().kind(native).is_none());
        assert_eq!((stats().page_builds, stats().setup_active), (1, 1));
        let hidden_setup = stats().setup_runs;
        page.send(Change::Value(7));
        page.settle();
        assert!(stats().setup_runs > hidden_setup);
        assert_eq!(stats().page_builds, 1);
        assert!(page.segment().scope().is_alive());
        page.set_presented(true);
        assert!(page.find_text("value 7").is_some());
        assert_eq!((stats().page_builds, stats().setup_active), (2, 1));
        let drawn = stats().setup_runs;
        page.set_presented(true);
        assert_eq!(
            stats().setup_runs,
            drawn,
            "duplicate native states must not redraw"
        );
        drop(page);
        assert_eq!((stats().setup_active, stats().setup_dropped), (0, 1));
    });
}

#[test]
fn an_initially_hidden_page_runs_setup_without_calling_the_body() {
    guinea_app::app::check(4, |h| {
        reset();
        guinea_app::app::windows::set_presentation(h.segment().scope(), false, false);
        let mut page = Mounted::<GatedPage>::mount(h.child(), ()).unwrap();
        assert_eq!((stats().setup_active, stats().page_builds), (1, 0));
        page.send(Change::Value(11));
        page.settle();
        assert_eq!(stats().page_builds, 0);
        page.set_presented(true);
        assert!(page.find_text("value 11").is_some());
    });
}

#[test]
fn layout_gate_keeps_mounts_outside_the_closure_alive_without_reinstalling() {
    guinea_app::app::check(4, |h| {
        reset();
        let mut layout = Mounted::<GatedLayout>::mount(h.child(), ()).unwrap();
        assert_eq!(stats().feedback_installs, 1);
        assert!(layout.find_text("feedback").is_some());
        layout.set_presented(false);
        assert!(layout.find_text("feedback").is_none());
        layout.send(9);
        layout.settle();
        assert_eq!(
            (
                stats().feedback_installs,
                stats().feedback_dropped,
                stats().layout_builds
            ),
            (1, 0, 1)
        );
        layout.set_presented(true);
        assert!(layout.find_text("layout 9").is_some());
        assert!(layout.find_text("feedback").is_some());
        assert_eq!((stats().feedback_installs, stats().setup_active), (1, 1));
        drop(layout);
        assert_eq!((stats().feedback_dropped, stats().setup_dropped), (1, 1));
    });
}

#[test]
fn omitting_the_gate_unsubscribes_and_reentering_reads_current_presentation() {
    guinea_app::app::check(4, |h| {
        reset();
        let mut page = Mounted::<GatedPage>::mount(h.child(), ()).unwrap();
        page.send(Change::Gate(false));
        page.settle();
        let draws = stats().setup_runs;
        page.set_presented(false);
        assert_eq!(
            stats().setup_runs,
            draws,
            "an omitted gate must stop listening"
        );
        page.send(Change::Gate(true));
        page.settle();
        assert!(page.find_text("value 0").is_none());
        page.set_presented(true);
        assert!(page.find_text("value 0").is_some());
    });
}

#[test]
fn gating_one_window_does_not_change_another_window() {
    guinea_app::app::check(4, |h| {
        reset();
        let other = guinea_app::app::roots::Root::under(&h.application());
        let mut a = Mounted::<GatedPage>::mount(h.child(), ()).unwrap();
        let mut b = Mounted::<GatedPage>::mount(h.over(other.scope()).child(), ()).unwrap();
        a.set_presented(false);
        b.settle();
        assert!(a.find_text("value 0").is_none());
        assert!(b.find_text("value 0").is_some());
        b.send(Change::Value(3));
        b.settle();
        assert!(b.find_text("value 3").is_some());
    });
}

struct ChangedBeforeSubscription(Scope);

#[page]
impl Page for ChangedBeforeSubscription {
    type Params = Scope;
    fn install(cx: &crate::Install<Scope>) -> anyhow::Result<Self> {
        Ok(Self(*cx.params()))
    }
    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let scope = self.0;
        cx.use_effect_guard("hide before watch is installed", (), move || {
            guinea_app::app::windows::set_presentation(scope, false, false);
        });
        cx.when_presented(|_| TextBlock::new().text("race body").into())
    }
}

#[test]
fn a_presentation_change_before_effect_commit_is_not_lost() {
    guinea_app::app::check(4, |h| {
        let scope = h.segment().scope();
        let mut page = Mounted::<ChangedBeforeSubscription>::mount(h.child(), scope).unwrap();
        assert!(page.find_text("race body").is_none());
        page.set_presented(true);
        assert!(page.find_text("race body").is_some());
    });
}
