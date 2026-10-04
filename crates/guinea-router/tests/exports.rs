//! What a feature publishes, and what stays its own.
//!
//! Before `Exports`, a reducer was reachable from anywhere below simply
//! because it existed - which made a feature a folder rather than a unit with
//! a surface. Now the surface is written down, and the two ends are asked
//! different questions: a segment may read whatever it claimed itself, and an
//! ancestor only what it listed.

use std::any::Any;
use std::rc::Rc;

use guinea_app::feature::{Feature, FeatureHost, FeatureInitContext, Segment};
use guinea_core::actor::UiThreadToken;
use guinea_core::feature::Bound;
use guinea_core::scope::Reducer;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{Router, SegmentEntry};

/// Published: pages below are meant to read this.
#[derive(Default, Clone, Debug)]
struct Shown(u32);

impl Reducer for Shown {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

/// Claimed by the same feature and deliberately not published - the feature's
/// own bookkeeping.
#[derive(Default, Clone, Debug)]
struct Hidden(u32);

impl Reducer for Hidden {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

thread_local! {
    /// Set while the feature exists, cleared when it is dropped.
    static LIVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Something only the feature holds - so whether it survives says whether the
/// feature did.
struct Alive;

impl Drop for Alive {
    fn drop(&mut self) {
        LIVING.with(|living| living.set(false));
    }
}

struct Chrome {
    _alive: Alive,
}

impl Feature for Chrome {
    type Params = ();
    /// One of the two. The other is claimed below and stays private.
    type Exports = (Shown,);

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        cx.state::<Shown>().seed(Shown(7)).plain();
        cx.state::<Hidden>().seed(Hidden(9)).plain();

        LIVING.with(|living| living.set(true));
        Ok(Self { _alive: Alive })
    }
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    /// Declared, and the body has to produce it.
    type Installs = Chrome;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Chrome> {
        ctx.install::<Chrome>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        // Without this the page below is never mounted, and a test that meant
        // to watch it read something would quietly watch nothing.
        cx.outlet();
    }
}

/// Reads what the layout published.
struct Reader;

impl Page for Reader {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        let (shown, _) = cx.read::<Shown, _>();
        assert_eq!(shown.0, 7);
    }
}

/// Claims its own `Hidden`, which is nobody else's business either - and its
/// own to read.
struct Owner;

impl Page for Owner {
    type Params = ();
    /// A reducer claimed directly rather than a feature - the claim is what
    /// `install` returns, so declaring it costs nothing extra.
    type Installs = Bound<Hidden>;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Bound<Hidden>> {
        Ok(ctx.state::<Hidden>().seed(Hidden(1)).plain())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        let (hidden, _) = cx.read::<Hidden, _>();
        assert_eq!(hidden.0, 1, "its own, not the layout's 9");
    }
}

// Where each segment sits. `routes!` writes these; a chain assembled by hand
// declares them by hand, and the two halves - who is above whom, and what each
// installs - are the same either way.
impl Segment for Shell {
    type Installs = <Shell as Layout>::Installs;
    type Above = ();
}

impl Segment for Reader {
    type Installs = <Reader as Page>::Installs;
    type Above = (Shell, ());
}

impl Segment for Owner {
    type Installs = <Owner as Page>::Installs;
    type Above = (Shell, ());
}

const READS: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Reader>()];
const OWNS: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Owner>()];

fn mounted(chain: &'static [SegmentEntry<Headless>]) -> Rc<Router<Headless>> {
    let token = UiThreadToken::dangerously_create_token_unchecked();
    let router = Rc::new(Router::<Headless>::new(FeatureHost::detached(token)));
    let params: Vec<Box<dyn Any>> = vec![Box::new(()), Box::new(())];
    router.activate(chain, params).expect("activate");
    router
}

#[test]
fn a_page_reads_what_its_layout_published() {
    mounted(&READS).render(&());
}

#[test]
fn a_feature_lives_as_long_as_the_segment_that_installed_it() {
    LIVING.with(|living| living.set(false));

    let router = mounted(&READS);
    assert!(
        LIVING.with(|living| living.get()),
        "what `install` returned is owned by the scope, not dropped at the end \
         of the call - a feature holding something nothing else holds would \
         otherwise die the moment it was built"
    );

    drop(router);
    assert!(
        !LIVING.with(|living| living.get()),
        "and it dies with the segment"
    );
}

/// Exporting something the feature never claimed.
struct Boaster;

impl Feature for Boaster {
    type Params = ();
    /// Declared and never claimed - the drift `Installs` closed on the other
    /// side of the feature.
    type Exports = (Shown,);

    fn install(_cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        Ok(Self)
    }
}

struct Boastful;

impl Layout for Boastful {
    type Params = ();
    type Installs = Boaster;

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Boaster> {
        ctx.install::<Boaster>(&())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

const BOASTS: [SegmentEntry<Headless>; 2] =
    [layout_entry::<Boastful>(), segment_entry::<Reader>()];

#[test]
#[should_panic(expected = "nothing in it claimed that reducer")]
fn exporting_what_was_never_claimed_is_caught_where_it_is_written() {
    // Not where it is read. A page below would otherwise get `Shown::default()`
    // - created on first read, so nothing fails - for as long as the
    // application runs, and it would look exactly like a feature that has not
    // pushed an update yet.
    mounted(&BOASTS).render(&());
}

#[test]
fn a_page_cannot_read_what_its_layout_kept_to_itself() {
    // Nothing to run: it stopped being a panic and became an error at the
    // read itself. The case lives as the `compile_fail` example on
    // `headless::HeadlessCx::state`, which is the only place it can be
    // checked - a test that does not compile is not a test.
}

#[derive(Default, Clone, Debug)]
struct Slot<const N: usize>(u32);

impl<const N: usize> Reducer for Slot<N> {
    type Update = u32;

    fn reduce(&mut self, to: u32) {
        self.0 = to;
    }
}

struct Only<const N: usize>;

impl<const N: usize> Feature for Only<N> {
    type Params = ();
    type Exports = (Slot<N>,);

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        cx.state::<Slot<N>>().seed(Slot(N as u32)).plain();
        Ok(Self)
    }
}

struct Wide;

impl Feature for Wide {
    type Params = ();
    type Exports = (Slot<10>, Slot<11>, Slot<12>, Slot<13>);

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        cx.state::<Slot<10>>().seed(Slot(10)).plain();
        cx.state::<Slot<11>>().seed(Slot(11)).plain();
        cx.state::<Slot<12>>().seed(Slot(12)).plain();
        cx.state::<Slot<13>>().seed(Slot(13)).plain();
        Ok(Self)
    }
}

struct Crowded;

impl Layout for Crowded {
    type Params = ();
    type Installs = (Only<1>, Only<2>, Wide, Only<3>);

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self::Installs> {
        Ok((
            ctx.install(&())?,
            ctx.install(&())?,
            ctx.install(&())?,
            ctx.install(&())?,
        ))
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

struct Beneath;

impl Page for Beneath {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        assert_eq!(cx.read::<Slot<1>, _>().0.0, 1);
        assert_eq!(cx.read::<Slot<13>, _>().0.0, 13);
        assert_eq!(cx.read::<Slot<3>, _>().0.0, 3);
    }
}

impl Segment for Crowded {
    type Installs = <Crowded as Layout>::Installs;
    type Above = ();
}

impl Segment for Beneath {
    type Installs = <Beneath as Page>::Installs;
    type Above = (Crowded, ());
}

const CROWDED: [SegmentEntry<Headless>; 2] =
    [layout_entry::<Crowded>(), segment_entry::<Beneath>()];

#[test]
fn installs_and_exports_are_flat_lists_of_any_length() {
    mounted(&CROWDED).render(&());
}

#[test]
fn a_segment_still_reads_whatever_it_claimed_itself() {
    // Unpublished does not mean unreadable - it means unreadable *from below*.
    // The page claimed its own, and gets its own rather than the layout's.
    mounted(&OWNS).render(&());
}
