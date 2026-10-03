//! Two instances of one feature in one scope.
//!
//! Reducers survive this on their own: `List<Recent>` and `List<Archived>` are
//! different types, so the scope's cells never collide. Actions do not - one
//! `Refresh` type, two features that answer it - and a scope keyed by the
//! action alone would let whichever installed last answer for both.
//!
//! Which is why a scope is not a flat map: each installed feature gets its own
//! section, and the dispatcher a reader is handed belongs to the section that
//! owns what it was reading.

use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use guinea_app::feature::{Feature, FeatureInitContext, Segment};
use guinea_core::actor::UiThreadToken;
use guinea_core::scope::Reducer;
use guinea_router::headless::{Headless, HeadlessCx, Layout, Page, layout_entry, segment_entry};
use guinea_router::router::{Router, SegmentEntry};

struct Refresh;

/// Which list. A marker, not a string - the same answer Riverpod reached, and
/// the one place every DI surveyed grew a stringly-typed escape hatch.
#[derive(Default, Debug)]
pub struct Recent;
#[derive(Default, Debug)]
pub struct Archived;

/// One reducer type per instance, for free: the marker is part of the type.
pub struct List<Which> {
    rows: Vec<String>,
    which: PhantomData<fn() -> Which>,
}

impl<Which> std::fmt::Debug for List<Which> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("List").field("rows", &self.rows).finish()
    }
}

impl<Which> Default for List<Which> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            which: PhantomData,
        }
    }
}

impl<Which> Clone for List<Which> {
    fn clone(&self) -> Self {
        Self {
            rows: self.rows.clone(),
            which: PhantomData,
        }
    }
}

impl<Which: 'static> Reducer for List<Which> {
    type Update = Vec<String>;

    fn reduce(&mut self, rows: Vec<String>) {
        self.rows = rows;
    }
}

thread_local! {
    /// Which instance actually answered, in order.
    static ANSWERED: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

pub struct Lists<Which> {
    which: PhantomData<fn() -> Which>,
}

/// What each instance calls itself when it answers.
trait Named: Default + 'static {
    const NAME: &'static str;
}

impl Named for Recent {
    const NAME: &'static str = "recent";
}

impl Named for Archived {
    const NAME: &'static str = "archived";
}

impl<Which: Named> Feature for Lists<Which> {
    type Params = ();
    type Exports = (List<Which>,);

    fn install(cx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self> {
        let rows = cx.state::<List<Which>>().plain();

        cx.answers::<Refresh>(move |_| {
            ANSWERED.with(|seen| seen.borrow_mut().push(Which::NAME));
            rows.push(vec![Which::NAME.to_string()]);
        });

        Ok(Self {
            which: PhantomData,
        })
    }
}

struct Shell;

impl Layout for Shell {
    type Params = ();
    type Installs = (Lists<Recent>, Lists<Archived>);

    fn install(ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<Self::Installs> {
        // Two instances, one scope, and the same action type between them.
        Ok((ctx.install(&())?, ctx.install(&())?))
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        cx.outlet();
    }
}

/// Asks each list to refresh, through the reducer it was reading.
struct Both;

impl Page for Both {
    type Params = ();
    type Installs = ();

    fn install(_ctx: &FeatureInitContext, _params: &()) -> anyhow::Result<()> {
        Ok(())
    }

    fn view(cx: &mut HeadlessCx<Self>) {
        let (_, recent) = cx.read::<List<Recent>, _>();
        let (_, archived) = cx.read::<List<Archived>, _>();

        recent.emit(Refresh);
        archived.emit(Refresh);
    }
}

impl Segment for Shell {
    type Installs = <Shell as Layout>::Installs;
    type Above = ();
}

impl Segment for Both {
    type Installs = <Both as Page>::Installs;
    type Above = (Shell, ());
}

const CHAIN: [SegmentEntry<Headless>; 2] = [layout_entry::<Shell>(), segment_entry::<Both>()];

#[test]
fn each_instance_answers_for_itself() {
    ANSWERED.with(|seen| seen.borrow_mut().clear());

    let token = UiThreadToken::dangerously_create_token_unchecked();
    let router = Rc::new(Router::<Headless>::new(token));
    let params: Vec<Box<dyn Any>> = vec![Box::new(()), Box::new(())];
    router.activate(&CHAIN, params).expect("activate");

    router.render(&());

    assert_eq!(
        ANSWERED.with(|seen| seen.borrow().clone()),
        ["recent", "archived"],
        "the dispatcher a reader is handed belongs to the feature that owns \
         what it read - not to whichever instance registered last"
    );

    let scope = router.scope_at(0).expect("the layout is mounted");
    assert_eq!(scope.state::<List<Recent>>().borrow().rows, ["recent"]);
    assert_eq!(scope.state::<List<Archived>>().borrow().rows, ["archived"]);
}
