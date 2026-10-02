use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

use guinea_core::binding::ReducerBinding;
use guinea_core::guard::{Ask, Decision, Verdict};
use guinea_core::scope::{Reducer, Scope};

use guinea_app::feature::{FeatureHost, FeatureInitContext};

use crate::enter::{EnterCx, EnterGuard};

/// A UI backend, described entirely by the types it works in.
///
/// Deliberately method-free: anything that looks like behaviour is carried as
/// data instead - mounting is a function pointer in [`SegmentEntry`],
/// navigation is a [`RouteSink`], identity is a [`SegmentIdentity`]. A trait
/// with methods here would have to name a common denominator of every
/// toolkit, and each backend would then be forced to fake the parts that do
/// not fit its model.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a guinea backend",
    note = "with more than one backend feature enabled there is no default one, so a route tree has to name it: `routes! {{ backend = guinea::winui::WinUi, .. }}` or `backend = guinea::ratatui::Tui`"
)]
pub trait Ui: Sized + 'static {
    /// What a view produces: an element tree for a reconciler, a deferred draw
    /// closure for immediate mode, `()` for a backend that pushes into
    /// retained properties.
    ///
    /// Borrowed from [`Nodes`](Self::Nodes) rather than owned, because some
    /// toolkits build widgets that hold a reference to the state they show -
    /// iced's `text_editor` keeps a `&Content` for the life of the element.
    /// A backend whose views own everything ignores the lifetime.
    type View<'a>;

    /// Where a backend keeps whatever its views borrow from.
    ///
    /// Threaded through mounting by reference, so a view can hold `&'a` into
    /// it. `()` for a backend that borrows nothing - which is four of the five
    /// here, and why this is an associated type rather than a requirement.
    type Nodes: 'static;
}

pub struct SegmentEntry<U: Ui> {
    pub type_id: fn() -> TypeId,
    pub type_name: fn() -> &'static str,
    /// What this segment captured, erased. The generated glue narrows it back
    /// to the segment's own params type; nothing hand-written sees `Any`.
    pub install: fn(&FeatureInitContext, &dyn Any) -> anyhow::Result<()>,
    /// Whether two captures for this segment are the same capture.
    ///
    /// Generated code rather than a trait bound on the route: it is the one
    /// question the router asks about parameters, and asking it here keeps
    /// `PartialEq` off the route type itself - which matters for a payload
    /// that has no identity to compare.
    pub same_params: fn(&dyn Any, &dyn Any) -> bool,
    /// Built by the backend: the agnostic half only calls it.
    pub mount: &'static dyn Mount<U>,
    pub cache_state: bool,
    /// Declared `keep` in `routes!`: left, it sleeps instead of being torn
    /// down, and a navigation back wakes it as it was.
    pub keep: bool,
    /// Where `routes!` listed this segment.
    pub declared: Option<guinea_core::actor::shape::Declared>,
    /// Where the segment itself was written, when `#[segment]` wrote it down.
    pub written: Option<guinea_core::actor::shape::Declared>,
}

/// How a segment turns into a view.
///
/// A trait object rather than a function pointer, and only because of a hole
/// in the compiler. A chain is `&'static`, so the entry cannot carry the
/// render's lifetime; a pointer would therefore have to be higher-ranked over
/// it - and rustc will not normalise `U::View<'a>` under a `for<'a>` binder,
/// so the coercion fails with "expected fn pointer, found fn item". A vtable
/// is normalised per call instead, and the entries stay `const`.
pub trait Mount<U: Ui>: 'static {
    fn view<'a>(&self, props: SegmentProps<U>, nodes: &'a U::Nodes) -> U::View<'a>;
}

/// Where a segment sits while it renders: the chain it belongs to, the scope
/// stack that chain installed, and which of the two this segment is.
pub struct SegmentProps<U: Ui> {
    pub chain: &'static [SegmentEntry<U>],
    pub scopes: Rc<Vec<Rc<Scope>>>,
    pub cursor: usize,
}

impl<U: Ui> Clone for SegmentProps<U> {
    fn clone(&self) -> Self {
        Self {
            chain: self.chain,
            scopes: self.scopes.clone(),
            cursor: self.cursor,
        }
    }
}

/// What makes a mounted segment *this* segment: where it sits in the chain,
/// what it is, and which scope it runs in.
///
/// Data rather than a trait method, because backends need it for different
/// things - a reconciler compares it to decide whether to re-mount, an
/// immediate-mode backend ignores it entirely. `derive`d rather than
/// hand-written so a field added later is compared automatically instead of
/// being silently forgotten.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIdentity {
    pub cursor: usize,
    pub segment: TypeId,
    pub scope: usize,
}

impl<U: Ui> PartialEq for SegmentProps<U> {
    fn eq(&self, other: &Self) -> bool {
        // A segment is its own identity *and* everything it renders below,
        // because that is what it draws: a layout emits its child through
        // `outlet()`. Comparing only this segment would let a reconciler skip
        // a layout whose own identity is unchanged - and skipping the layout
        // skips the `outlet()` call, so navigating between two pages under a
        // shared layout would leave the old page on screen.
        //
        // Note what is deliberately not compared. Not
        // `std::ptr::eq(self.chain, other.chain)`: sibling leaves under the
        // same layout each get their own `const` chain array (see
        // `routes_dsl.rs`), so the arrays always differ even when the segment
        // is unchanged. Not `Rc::ptr_eq(&self.scopes, &other.scopes)` either:
        // `install_from` wraps the whole stack in a fresh `Rc` on every
        // navigation, even when the individual scopes are reused.
        self.cursor == other.cursor && self.tail_identity().eq(other.tail_identity())
    }
}

impl<U: Ui> SegmentProps<U> {
    /// This segment's identity, then every segment it renders inside itself.
    pub fn tail_identity(&self) -> impl Iterator<Item = SegmentIdentity> + '_ {
        let chain = self.chain;
        let scopes = &self.scopes;
        (self.cursor..chain.len().min(scopes.len())).map(move |cursor| SegmentIdentity {
            cursor,
            segment: (chain[cursor].type_id)(),
            scope: Rc::as_ptr(&scopes[cursor]) as usize,
        })
    }

    pub fn identity(&self) -> SegmentIdentity {
        SegmentIdentity {
            cursor: self.cursor,
            segment: (self.chain[self.cursor].type_id)(),
            scope: Rc::as_ptr(&self.scopes[self.cursor]) as usize,
        }
    }

    /// The scope that owns `R`, and a binding to its state.
    ///
    /// This segment's own scope counts whatever it claimed; an ancestor counts
    /// only what it *exported*. A feature's internal state used to be
    /// reachable from anywhere below simply because it existed - now it is
    /// reachable exactly where the feature said, and `Exports` is where it
    /// says so.
    ///
    /// A miss is a setup bug rather than a race: ownership is recorded
    /// synchronously during `install()`, always before the first render for
    /// this segment. So it panics rather than silently treating the current
    /// scope as the owner.
    pub fn binding<R: Reducer>(&self) -> ReducerBinding<R> {
        let owner = resolve::<R>(&self.scopes[..=self.cursor]).unwrap_or_else(|| {
            panic!(
                "reading {} here found no scope that owns it: this segment did not claim \
                 it, and no ancestor exported it. Either this route never installs the \
                 feature that owns it, that feature installs in another branch, or it \
                 owns the reducer without listing it in `Exports`.",
                std::any::type_name::<R>()
            )
        });
        owner.binding::<R>()
    }

    /// Mounts the next segment down the chain. Backends expose this to layouts
    /// only: a page is the end of the chain and has no child to render.
    ///
    /// The child borrows from the same `nodes` the parent does, which is what
    /// makes the whole tree one borrow of the backend's store rather than a
    /// chain of temporaries.
    pub fn outlet<'a>(&self, nodes: &'a U::Nodes) -> U::View<'a> {
        let next = self.cursor + 1;
        assert!(
            next < self.chain.len(),
            "outlet() called on the last segment of the chain (no child to render)"
        );
        self.chain[next].mount.view(
            SegmentProps {
                chain: self.chain,
                scopes: self.scopes.clone(),
                cursor: next,
            },
            nodes,
        )
    }
}

/// Finds the scope that owns `R` for a segment sitting at the end of `chain`.
///
/// Innermost first, and the two ends are asked different questions: the
/// segment itself may read anything it claimed, an ancestor only what it
/// exported.
pub fn resolve<R: Reducer>(chain: &[Rc<Scope>]) -> Option<&Rc<Scope>> {
    let (here, above) = chain.split_last()?;
    if here.has_feature::<R>() {
        return Some(here);
    }
    above.iter().rev().find(|scope| Scope::exports::<R>(scope))
}

/// Narrows what the router carries back to what a segment declared.
///
/// For the glue a backend generates around `install`; a mismatch means the
/// chain and the route were built from different declarations, which is a
/// wiring bug rather than anything a user did.
pub fn narrow<'a, T: 'static, Segment: 'static>(
    params: &'a dyn Any,
) -> anyhow::Result<&'a T> {
    params.downcast_ref::<T>().ok_or_else(|| {
        anyhow::anyhow!(
            "`{}` expects params of type `{}`, but the route carried something else",
            std::any::type_name::<Segment>(),
            std::any::type_name::<T>(),
        )
    })
}

/// Whether two captures for the same segment are the same capture.
///
/// For the glue a backend generates around `install`, next to [`narrow`].
pub fn same_params<T: PartialEq + 'static>(left: &dyn Any, right: &dyn Any) -> bool {
    match (left.downcast_ref::<T>(), right.downcast_ref::<T>()) {
        (Some(left), Some(right)) => left == right,
        // Different types in the same position means the two chains were built
        // from different declarations. Not the same place, so not the same
        // capture either.
        _ => false,
    }
}

impl<U: Ui> SegmentEntry<U> {
    /// Built by the backend, which is where "page" and "layout" are defined -
    /// the router only needs something it can install and mount.
    pub const fn new<S: 'static>(
        install: fn(&FeatureInitContext, &dyn Any) -> anyhow::Result<()>,
        same_params: fn(&dyn Any, &dyn Any) -> bool,
        mount: &'static dyn Mount<U>,
        cache_state: bool,
    ) -> Self {
        Self {
            type_id: TypeId::of::<S>,
            type_name: std::any::type_name::<S>,
            install,
            same_params,
            mount,
            cache_state,
            keep: false,
            declared: None,
            written: None,
        }
    }

    /// The same entry, kept asleep when a navigation leaves it.
    pub const fn kept(self) -> Self {
        Self { keep: true, ..self }
    }

    /// The same entry, knowing where `routes!` listed it.
    pub const fn at(self, declared: guinea_core::actor::shape::Declared) -> Self {
        Self {
            declared: Some(declared),
            ..self
        }
    }

    /// The same entry, knowing where the segment itself was written.
    pub const fn written(self, written: Option<guinea_core::actor::shape::Declared>) -> Self {
        Self { written, ..self }
    }
}

/// A one-segment chain for `P`, leaked once per page type.
///
/// Leaked rather than owned because `SegmentEntry` chains are `&'static` -
/// `routes!` generates them as `const` arrays, and this is the path for a page
/// activated without a route tree (tests, and `Router::activate`).
pub fn single_entry_chain<U: Ui>(entry: SegmentEntry<U>) -> &'static [SegmentEntry<U>] {
    Box::leak(Box::new([entry])) as &'static [SegmentEntry<U>]
}

pub trait RouteChain<U: Ui> {
    fn chain(&self) -> &'static [SegmentEntry<U>];

    /// What each segment of the chain captured, in chain order and the same
    /// length as [`chain`](Self::chain).
    fn params(&self) -> Vec<Box<dyn Any>>;

    /// What to call this route in a log or a navigation hook - present even
    /// for a route nothing outside the application can name.
    fn name(&self) -> &'static str;

    /// The route with what it carries, for devtools. `routes!` prints it
    /// with `Debug` where every field allows that.
    fn describe(&self) -> String {
        self.name().to_string()
    }

    /// The address this route answers to, when it agreed to have one.
    fn link(&self) -> Option<String> {
        None
    }

    /// What stands in front of this route, outermost declaration first.
    ///
    /// Already resolved by `routes!`: the cascade is folded in and the
    /// opt-outs are taken back out, so this is the list as it applies here
    /// rather than a tree to walk. The router asks the route because that is
    /// where enter guards are declared - a `SegmentEntry` is built by the
    /// backend from a page type, which knows nothing about where in a tree it
    /// was placed.
    ///
    /// Defaulted, so a chain written by hand - a test, a page mounted without
    /// a tree - says nothing and is guarded by nothing.
    fn guards(&self) -> &'static [&'static dyn EnterGuard] {
        &[]
    }
}

/// Where a navigation goes once the router has accepted it - a reconciler's
/// state setter, a field in a TUI's application struct, a Slint property.
///
/// Data, not a trait: the agnostic half publishes the new route, the backend
/// decides what publishing means.
/// Keeps a route-change hook alive. Dropping it removes the hook.
pub struct RouteHookHandle {
    id: usize,
    router: std::rc::Weak<dyn AnyRouter>,
}

impl Drop for RouteHookHandle {
    fn drop(&mut self) {
        if let Some(router) = self.router.upgrade() {
            router.remove_route_hook(self.id);
        }
    }
}

/// The part of `Router` a hook handle needs, without its backend parameter -
/// a handle should not have to name `U` just to unregister itself.
pub trait AnyRouter {
    fn remove_route_hook(&self, id: usize);
}

impl<U: Ui> AnyRouter for Router<U> {
    fn remove_route_hook(&self, id: usize) {
        self.route_hooks.borrow_mut().retain(|(this, _)| *this != id);
        self.question_hooks.borrow_mut().retain(|(this, _)| *this != id);
    }
}

pub struct RouteSink<R> {
    publish: Rc<dyn Fn(R)>,
}

impl<R> RouteSink<R> {
    pub fn new(publish: impl Fn(R) + 'static) -> Self {
        Self {
            publish: Rc::new(publish),
        }
    }

    pub fn publish(&self, route: R) {
        (self.publish)(route)
    }
}

impl<R> Clone for RouteSink<R> {
    fn clone(&self) -> Self {
        Self {
            publish: self.publish.clone(),
        }
    }
}

/// Which stack a refused step has to be put back on.
#[derive(Clone, Copy)]
enum Step {
    Back,
    Forward,
}

pub struct NavigateHandle<U: Ui, R> {
    /// `None` for a [`recording`](NavigateHandle::recording) handle, which
    /// has no router to move.
    router: Option<Rc<Router<U>>>,
    sink: RouteSink<R>,
}

impl<U: Ui, R> Clone for NavigateHandle<U, R> {
    fn clone(&self) -> Self {
        Self {
            router: self.router.clone(),
            sink: self.sink.clone(),
        }
    }
}

impl<U: Ui, R> PartialEq for NavigateHandle<U, R> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.router, &other.router) {
            (Some(mine), Some(theirs)) => Rc::ptr_eq(mine, theirs),
            (None, None) => Rc::ptr_eq(&self.sink.publish, &other.sink.publish),
            _ => false,
        }
    }
}

impl<U: Ui, R> NavigateHandle<U, R>
where
    R: RouteChain<U> + Clone + PartialEq + 'static,
{
    pub fn new(router: Rc<Router<U>>, sink: RouteSink<R>) -> Self {
        Self {
            router: Some(router),
            sink,
        }
    }

    /// A handle that moves no router: every [`to`](Self::to) goes straight to
    /// `sink`, and there is no history to step through. For a harness, which
    /// mounts a segment with no route tree and wants to know where it asked
    /// to go.
    pub fn recording(sink: RouteSink<R>) -> Self {
        Self { router: None, sink }
    }

    /// Go to `route`, recording where we were.
    ///
    /// History moves only if the navigation actually happens: a guard that
    /// refuses must not leave the place we never left sitting in the back
    /// stack.
    pub fn to(&self, route: R) {
        let Some(router) = &self.router else {
            self.sink.publish(route);
            return;
        };

        let leaving = self.current().map(|route| Box::new(route) as Box<dyn Any>);
        let weak = Rc::downgrade(router);

        self.go(router, route, move |arrived| {
            if !arrived {
                return;
            }
            if let (Some(router), Some(leaving)) = (weak.upgrade(), leaving) {
                router.remember(leaving);
            }
        });
    }

    /// Back one step, if there is one. Reports whether it moved, so a key
    /// handler can fall through to something else - closing a dialog, or
    /// quitting - when there is nowhere to go.
    ///
    /// A guard can defer this, in which case it reports `false` now and moves
    /// later - the answer is not available to report on.
    pub fn back(&self) -> bool {
        let Some(router) = &self.router else {
            return false;
        };

        let leaving = self.current().map(|route| Box::new(route) as Box<dyn Any>);
        let Some(entry) = router.take_back(leaving) else {
            return false;
        };
        self.arrive(router, entry, Step::Back)
    }

    /// Forward one step, undoing a [`back`](Self::back).
    pub fn forward(&self) -> bool {
        let Some(router) = &self.router else {
            return false;
        };

        let leaving = self.current().map(|route| Box::new(route) as Box<dyn Any>);
        let Some(entry) = router.take_forward(leaving) else {
            return false;
        };
        self.arrive(router, entry, Step::Forward)
    }

    pub fn can_go_back(&self) -> bool {
        self.router.as_ref().is_some_and(|router| router.can_go_back())
    }

    pub fn can_go_forward(&self) -> bool {
        self.router
            .as_ref()
            .is_some_and(|router| router.can_go_forward())
    }

    /// Where the router is now, as this handle's route type.
    fn current(&self) -> Option<R> {
        self.router.as_ref()?.current_route::<R>()
    }

    fn arrive(&self, router: &Rc<Router<U>>, entry: Visited, step: Step) -> bool {
        let Some(route) = entry.route::<R>() else {
            // Two route types on one router. Nothing in guinea builds that,
            // and silently doing nothing beats navigating somewhere wrong.
            tracing::warn!("history holds a route of another type; ignoring it");
            return false;
        };

        // The entry is already out of the stack, so a refusal has to put it
        // back - otherwise a guarded page would eat a step of history every
        // time it said no.
        let weak = Rc::downgrade(router);
        let undo = route.clone();
        self.go(router, route, move |arrived| {
            if arrived {
                return;
            }
            if let Some(router) = weak.upgrade() {
                let entry = Box::new(undo) as Box<dyn Any>;
                match step {
                    Step::Back => router.restore_back(entry),
                    Step::Forward => router.restore_forward(entry),
                }
            }
        })
    }

    /// Reports whether it arrived *now*. A deferred navigation reports
    /// `false`: there is no answer yet to report.
    ///
    /// A segment that fails to install is logged and reported as not
    /// arrived: the handle is called from a click, and the router has
    /// already gone back to where it was.
    fn go(
        &self,
        router: &Rc<Router<U>>,
        route: R,
        settled: impl FnOnce(bool) + 'static,
    ) -> bool {
        let name = route.name();
        let weak = Rc::downgrade(router);
        let sink = self.sink.clone();
        let published = route.clone();

        let outcome = router.navigate_then(route, move |arrived| {
            if arrived {
                if let Some(router) = weak.upgrade() {
                    router.route_changed(name);
                }
                sink.publish(published);
            }
            settled(arrived);
        });

        match outcome {
            Ok(outcome) => outcome.is_done(),
            Err(error) => {
                tracing::error!(error = format!("{error:#}"), route = name, "navigation failed");
                false
            }
        }
    }

    /// A parameterless handler that navigates to `route` - sugar for
    /// `move || nav.to(route.clone())`, matching `SetState::setter`'s own
    /// convention. Handy for `on_click`-style events wired to a fixed route,
    /// e.g. `button("Processes").on_click(nav.to_handler(Route::Processes { .. }))`.
    pub fn to_handler(&self, route: R) -> impl Fn() + Clone + 'static {
        let nav = self.clone();
        move || nav.to(route.clone())
    }
}

/// This route type's `Context` for `NavigateHandle<R>` - the same `Context`
/// value (same `ContextId`) on every call for a given `R`, so a provider and
/// any consumer agree on which slot they're reading/writing. Only the `id`
/// needs to be process-stable (kept in a `TypeId`-keyed cache, not a bare
/// `static` inside this generic fn, which would be a single symbol shared
/// across every `R` - the same footgun `single_entry_chain` works around);
/// `default` is rebuilt fresh each call, cheaply, since it's only a fallback
/// for "nothing provided yet". Private: `RouterRx::render` is the only
/// provider, `RenderCx::use_navigate` the only consumer - nothing else needs
/// the raw `Context`.
pub(crate) struct ActiveChain<U: Ui> {
    pub(crate) entries: &'static [SegmentEntry<U>],
    pub(crate) scopes: Rc<Vec<Rc<Scope>>>,
    /// What each segment was installed with, kept so the next navigation can
    /// ask which of them changed - and so a cached state can refuse to come
    /// back to a segment that captured something else.
    pub(crate) params: Vec<Box<dyn Any>>,
    /// How to make them again, when the chain came from a route.
    again: Option<Again>,
}

impl<U: Ui> ActiveChain<U> {
    fn root_view<'a>(&self, nodes: &'a U::Nodes) -> U::View<'a> {
        let props = SegmentProps {
            chain: self.entries,
            scopes: self.scopes.clone(),
            cursor: 0,
        };
        self.entries[0].mount.view(props, nodes)
    }
}

/// Dropping a router is a teardown like any other, and has to run in the same
/// direction. Without this the field just drops, which takes the chain apart
/// from the outside in - the case that is hardest to notice, since it is the
/// one a closing window takes.
impl<U: Ui> Drop for Router<U> {
    fn drop(&mut self) {
        self.forget_sleeping();

        if let Ok(mut active) = self.active.try_borrow_mut()
            && let Some(active) = active.take()
        {
            unwind(active.scopes, 0);
        }

        if self.listed.get() {
            crate::devtools::unregister(self.root());
        }
    }
}

/// Tears a chain down to its first `keep` segments and hands back what is
/// left standing.
///
/// Innermost first, which is the direction the declarations point: a segment
/// says what it installs and what it reads from above, so the one that reads
/// is the one that has to go before what it reads. Dropping the vector - or
/// `Vec::truncate`, which was here before - runs the other way and tears a
/// layout down while the page inside it is still being torn down.
///
/// It degraded quietly rather than crashing, because [`Push`] holds its scope
/// weakly and a late update lands nowhere. That made it invisible, not
/// harmless: a teardown that touched what it depended on found it gone.
///
/// [`Push`]: guinea_core::feature::Push
fn unwind(scopes: Rc<Vec<Rc<Scope>>>, keep: usize) -> Vec<Rc<Scope>> {
    let mut standing = match Rc::try_unwrap(scopes) {
        Ok(owned) => owned,
        // Something still holds the chain - a view mid-render, say. Those
        // scopes die with it; the ones this owns still go in the right order.
        Err(shared) => (*shared).clone(),
    };

    while standing.len() > keep {
        standing.pop();
    }
    standing
}

fn common_prefix_len<U: Ui>(prev: &[SegmentEntry<U>], next: &[SegmentEntry<U>]) -> usize {
    prev.iter()
        .zip(next.iter())
        .take_while(|(a, b)| (a.type_id)() == (b.type_id)())
        .count()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct StateCacheKey {
    segment_index: usize,
    /// Every segment type from the root down to this one, hashed.
    ///
    /// Not this segment's type alone: the same page under two different
    /// layouts sits at the same depth and is not the same place, and keying on
    /// depth and type only would let one restore into the other.
    path: u64,
}

/// Every segment type from the root down to `index`, hashed.
///
/// Public because a backend keeping state of its own per segment has the same
/// question to answer, and answering it differently is how one page's state
/// ends up under another.
pub fn placement_hash<U: Ui>(chain: &[SegmentEntry<U>], index: usize) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in &chain[..=index] {
        (entry.type_id)().hash(&mut hasher);
    }
    hasher.finish()
}

/// Where a segment sits, as a key.
fn cache_key<U: Ui>(chain: &[SegmentEntry<U>], index: usize) -> StateCacheKey {
    StateCacheKey {
        segment_index: index,
        path: placement_hash(chain, index),
    }
}

const MAX_CACHED_STATES: usize = 10;

/// A segment's states, and what it had captured when they were taken.
struct Cached {
    states: HashMap<TypeId, Rc<dyn Any>>,
    params: Box<dyn Any>,
}

struct StateCache {
    entries: HashMap<StateCacheKey, Cached>,
    /// Order of insertion for LRU eviction.
    order: VecDeque<StateCacheKey>,
}

impl StateCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, key: StateCacheKey, cached: Cached) {
        if self.entries.insert(key, cached).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_CACHED_STATES {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    fn take(&mut self, key: StateCacheKey) -> Option<Cached> {
        if self.entries.contains_key(&key) {
            self.order.retain(|k| *k != key);
        }
        self.entries.remove(&key)
    }
}

/// A `keep` segment a navigation left: its scope, asleep, and what it needs
/// to be woken as the same segment.
struct Sleeping {
    scope: Rc<Scope>,
    /// What it had captured. Woken only for the same capture.
    params: Box<dyn Any>,
    /// The scopes it was installed under, outermost first. It wakes only
    /// below the very same ones: what it inherited from them is theirs.
    above: Vec<Weak<Scope>>,
}

impl Sleeping {
    fn is_below(&self, scope: &Rc<Scope>) -> bool {
        self.above
            .iter()
            .any(|above| std::ptr::eq(above.as_ptr(), Rc::as_ptr(scope)))
    }

    fn sits_on(&self, standing: &[Rc<Scope>]) -> bool {
        self.above.len() == standing.len()
            && self
                .above
                .iter()
                .zip(standing)
                .all(|(above, scope)| std::ptr::eq(above.as_ptr(), Rc::as_ptr(scope)))
    }
}

/// How far below `shared_len` a navigation leaving `entries` keeps them: the
/// run of `keep` segments right under what stays. A `keep` segment below one
/// that goes is torn down with it - it has nothing left to wake under.
fn kept_until<U: Ui>(entries: &[SegmentEntry<U>], shared_len: usize) -> usize {
    shared_len
        + entries
            .iter()
            .skip(shared_len)
            .take_while(|entry| entry.keep)
            .count()
}

/// A place the application has been: the route itself, erased, and the path
/// it produced.
pub struct Visited {
    route: Box<dyn Any>,
}

impl Visited {
    /// The route, if it is the type asked for. Anything else means two route
    /// types shared one router, which nothing in guinea sets up.
    pub fn route<R: 'static>(self) -> Option<R> {
        self.route.downcast::<R>().ok().map(|route| *route)
    }
}

/// What became of a navigation.
///
/// Three outcomes rather than two, because a guard may not have an answer yet.
/// A navigation waiting for one has changed nothing, which is what makes it
/// safe to supersede.
pub enum Navigation {
    /// Installed. The leaf's scope.
    Done(Rc<Scope>),
    /// A guard is asking. Nothing has changed; see [`Router::pending`].
    Deferred,
    /// A guard refused.
    Blocked,
}

impl Navigation {
    pub fn scope(self) -> Option<Rc<Scope>> {
        match self {
            Navigation::Done(scope) => Some(scope),
            _ => None,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Navigation::Done(_))
    }
}

/// A navigation waiting on an answer.
struct Parked<U: Ui> {
    ask: Ask,
    decision: Decision,
    chain: &'static [SegmentEntry<U>],
    params: Vec<Box<dyn Any>>,
    shared_len: usize,
    arrival: Arrival,
    /// What the caller wanted done once this either happened or did not.
    settled: Box<dyn FnOnce(bool)>,
}

/// What a chain's route can make its params again with - to put the chain
/// back when the navigation that replaced it fails to install.
type Again = Rc<dyn Fn() -> Vec<Box<dyn Any>>>;

/// The route a navigation is going to, kept aside until it has arrived: the
/// router says it is somewhere only once it is.
struct Arrival {
    route: Rc<dyn Any>,
    described: String,
    again: Again,
}

impl Arrival {
    fn of<U: Ui, R: RouteChain<U> + 'static>(route: R) -> Self {
        let described = route.describe();
        let route = Rc::new(route);

        let making = route.clone();
        Self {
            route,
            described,
            again: Rc::new(move || making.params()),
        }
    }
}

/// Says a frame is in progress for as long as it is held; see
/// [`Router::drawing`].
#[must_use = "the frame is over as soon as this is dropped"]
pub struct Drawing<'a, U: Ui> {
    router: &'a Router<U>,
}

impl<U: Ui> Drop for Drawing<'_, U> {
    fn drop(&mut self) {
        self.router.drawing.set(false);
    }
}

/// A navigation asked for while the chain it would replace was being drawn.
struct Held<U: Ui> {
    chain: &'static [SegmentEntry<U>],
    params: Vec<Box<dyn Any>>,
    shared_len: usize,
    arrival: Arrival,
    settled: Box<dyn FnOnce(bool)>,
}

pub struct Router<U: Ui> {
    pub(crate) active: RefCell<Option<ActiveChain<U>>>,
    /// The question on screen, if a guard asked one.
    pending: RefCell<Option<Parked<U>>>,
    /// Whether a frame is being drawn from the active chain right now, and
    /// what was asked for while it was.
    drawing: std::cell::Cell<bool>,
    held: RefCell<Option<Held<U>>>,
    /// Bumped by every navigation, so an answer to a superseded question can
    /// be told from an answer to the current one.
    generation: std::cell::Cell<u64>,
    prev_route: RefCell<Option<Rc<dyn Any>>>,
    /// Where the application has been, and where it was pulled back from.
    ///
    /// Kept here rather than in the application because the browser is the
    /// only shell that supplies a history: WinUI and a terminal have none, and
    /// every one of them would otherwise grow the same stack next to the
    /// router. Routes are held erased - the router never learns `R` - and
    /// [`NavigateHandle`] downcasts them back on the way out.
    back: RefCell<Vec<Visited>>,
    forward: RefCell<Vec<Visited>>,
    /// Installing a feature is not a routing concern - the router asks the
    /// host for a context and does the part that is its own: which scopes to
    /// keep, which to tear down, and in what order.
    host: FeatureHost,
    state_cache: RefCell<StateCache>,
    /// The `keep` segments navigations have left, by where they sat.
    sleeping: RefCell<HashMap<StateCacheKey, Sleeping>>,
    /// Notified after a navigation is applied. Kept here rather than on the
    /// application, because a route change is something only a router has -
    /// an application without one has nothing to report.
    route_hooks: RefCell<Vec<(usize, Rc<dyn Fn(Option<&str>, &str)>)>>,
    /// Notified when the pending question changes, for a backend that draws
    /// only when told to.
    question_hooks: RefCell<Vec<(usize, Rc<dyn Fn()>)>>,
    next_hook_id: std::cell::Cell<usize>,
    last_route: RefCell<Option<String>>,
    /// The mounted route as [`RouteChain::describe`] put it, for devtools.
    described: RefCell<Option<String>>,
    listed: std::cell::Cell<bool>,
}

impl<U: Ui> Router<U> {
    pub fn new(token: guinea_core::actor::UiThreadToken) -> Self {
        Self::with_host(FeatureHost::new(token))
    }

    /// Which root this router belongs to.
    ///
    /// One per host, so routers sharing a host share a root - and a router
    /// with a host of its own is a root of its own, which is what a second
    /// window gets.
    pub fn root(&self) -> guinea_app::app::roots::RootId {
        self.host.root()
    }

    /// For a caller that already has a host - one window hosting more than a
    /// single router, say, where features must share an event bus.
    pub fn with_host(host: FeatureHost) -> Self {
        Self {
            active: RefCell::new(None),
            pending: RefCell::new(None),
            drawing: std::cell::Cell::new(false),
            held: RefCell::new(None),
            generation: std::cell::Cell::new(0),
            prev_route: RefCell::new(None),
            back: RefCell::new(Vec::new()),
            forward: RefCell::new(Vec::new()),
            host,
            state_cache: RefCell::new(StateCache::new()),
            sleeping: RefCell::new(HashMap::new()),
            route_hooks: RefCell::new(Vec::new()),
            question_hooks: RefCell::new(Vec::new()),
            next_hook_id: std::cell::Cell::new(0),
            last_route: RefCell::new(None),
            described: RefCell::new(None),
            listed: std::cell::Cell::new(false),
        }
    }

    /// Runs `hook` after each navigation, with the previous path (`None` for
    /// the first) and the new one.
    ///
    /// The hook lasts as long as the returned handle. A caller that wants it
    /// for the life of the router can `std::mem::forget` it, but the usual
    /// caller is a view that must stop listening when it unmounts.
    #[must_use = "the hook is removed when the handle is dropped"]
    pub fn on_route_change(
        self: &Rc<Self>,
        hook: impl Fn(Option<&str>, &str) + 'static,
    ) -> RouteHookHandle {
        let id = self.next_hook_id.get();
        self.next_hook_id.set(id + 1);
        self.route_hooks.borrow_mut().push((id, Rc::new(hook)));
        RouteHookHandle {
            id,
            router: Rc::downgrade(&(self.clone() as Rc<dyn AnyRouter>)),
        }
    }

    /// Runs `hook` whenever the question a guard is waiting on changes: one
    /// is asked, answered, or dropped for a later navigation.
    /// [`pending`](Self::pending) says what it is now.
    ///
    /// For a backend that draws only when told to - a reconciler, a retained
    /// toolkit. One that draws every frame reads `pending` as it goes.
    #[must_use = "the hook is removed when the handle is dropped"]
    pub fn on_question(self: &Rc<Self>, hook: impl Fn() + 'static) -> RouteHookHandle {
        let id = self.next_hook_id.get();
        self.next_hook_id.set(id + 1);
        self.question_hooks.borrow_mut().push((id, Rc::new(hook)));
        RouteHookHandle {
            id,
            router: Rc::downgrade(&(self.clone() as Rc<dyn AnyRouter>)),
        }
    }

    fn questioned(&self) {
        let hooks: Vec<_> = self
            .question_hooks
            .borrow()
            .iter()
            .map(|(_, hook)| hook.clone())
            .collect();
        for hook in hooks {
            hook();
        }
    }

    /// Announces a navigation to the hooks. Called by `NavigateHandle::to`
    /// once the router has accepted the route.
    pub fn route_changed(&self, to: &str) {
        let from = self.last_route.borrow().clone();
        // Cloned out before calling: a hook is free to navigate, and would
        // otherwise re-enter this borrow.
        let hooks: Vec<_> = self
            .route_hooks
            .borrow()
            .iter()
            .map(|(_, hook)| hook.clone())
            .collect();
        for hook in hooks {
            hook(from.as_deref(), to);
        }
        *self.last_route.borrow_mut() = Some(to.to_string());
    }

    pub fn host(&self) -> &FeatureHost {
        &self.host
    }

    pub(crate) fn described(&self) -> Option<String> {
        self.described.borrow().clone()
    }

    pub(crate) fn history_len(&self) -> (usize, usize) {
        (self.back.borrow().len(), self.forward.borrow().len())
    }
    
    /// Installs a chain directly, outside any route tree - one segment, as a
    /// rule. The backend builds the chain, since only it knows how a segment
    /// is mounted, and hands over what each segment captured.
    pub fn activate(
        &self,
        chain: &'static [SegmentEntry<U>],
        params: Vec<Box<dyn Any>>,
    ) -> anyhow::Result<Rc<Scope>> {
        *self.prev_route.borrow_mut() = None;
        self.install_from(chain, 0, params, None)
    }

    pub fn navigate<R>(self: &Rc<Self>, route: R) -> anyhow::Result<Navigation>
    where
        R: RouteChain<U> + 'static,
    {
        self.navigate_then(route, |_| {})
    }

    /// Navigate, and run `settled` when the outcome is known.
    ///
    /// A callback rather than the return value alone, because a guard can park
    /// the navigation: whatever the caller meant to do on arrival - announce
    /// the route, publish it, move history - has to survive until the answer.
    /// `settled(false)` runs when a guard refuses, and when a later navigation
    /// supersedes this one.
    pub fn navigate_then<R>(
        self: &Rc<Self>,
        route: R,
        settled: impl FnOnce(bool) + 'static,
    ) -> anyhow::Result<Navigation>
    where
        R: RouteChain<U> + 'static,
    {
        if !self.listed.replace(true) {
            crate::devtools::register(self);
        }
        let _navigating = guinea_core::trace::enter(|| guinea_core::trace::Point::Navigate {
            root: guinea_app::app::roots::label(self.root())
                .unwrap_or_else(|| self.root().to_string()),
            to: route.name().to_string(),
        });

        let generation = self.generation.get() + 1;
        self.generation.set(generation);

        // A question still on screen belongs to a navigation nobody wants any
        // more. Last intent wins, and it is safe precisely because deciding
        // mutated nothing.
        let superseded = self.pending.borrow_mut().take();
        if let Some(parked) = superseded {
            (parked.settled)(false);
            self.questioned();
        }

        let chain = route.chain();
        let params = route.params();
        let shared_len = self.shared_len(chain, &params);

        // Entering is asked before leaving, and the reason is not the order
        // between guards of one direction - it is that a question about
        // unsaved work is pointless when the destination is going to refuse
        // anyway. Asking "discard changes?" and then blocking is a worse
        // answer than blocking.
        let verdict = match self.may_enter(route.guards(), route.name()) {
            Verdict::Allow => self.may_leave(shared_len),
            refused => refused,
        };

        match verdict {
            Verdict::Allow => {
                let arrival = Arrival::of(route);

                if self.drawing.get() {
                    // The frame is standing on the chain this would tear
                    // down, and holds it alive: installing now would run
                    // every teardown against scopes nothing can drop yet.
                    // It goes in as soon as the frame is over.
                    let superseded = self.held.borrow_mut().replace(Held {
                        chain,
                        params,
                        shared_len,
                        arrival,
                        settled: Box::new(settled),
                    });
                    if let Some(held) = superseded {
                        (held.settled)(false);
                    }

                    return Ok(Navigation::Deferred);
                }

                match self.commit(chain, shared_len, params, arrival) {
                    Ok(leaf) => {
                        settled(true);
                        Ok(Navigation::Done(leaf))
                    }
                    Err(error) => {
                        settled(false);
                        Err(error)
                    }
                }
            }

            Verdict::Block => {
                settled(false);
                Ok(Navigation::Blocked)
            }

            Verdict::Ask(ask, decision) => {
                self.pending.borrow_mut().replace(Parked {
                    ask,
                    decision: decision.clone(),
                    chain,
                    params,
                    shared_len,
                    arrival: Arrival::of(route),
                    settled: Box::new(settled),
                });

                // Wired only now, and weakly: a guard is allowed to settle its
                // own token before returning, so this may run at once - and
                // the parked entry has to be in place before it does.
                let router = Rc::downgrade(self);
                decision.on_answer(move |allowed| {
                    let Some(router) = router.upgrade() else {
                        return;
                    };
                    if router.generation.get() != generation {
                        return;
                    }
                    let Some(parked) = router.pending.borrow_mut().take() else {
                        return;
                    };
                    router.questioned();

                    if !allowed {
                        (parked.settled)(false);
                        return;
                    }

                    match router.commit(parked.chain, parked.shared_len, parked.params, parked.arrival) {
                        Ok(_) => (parked.settled)(true),
                        Err(error) => {
                            tracing::error!(%error, "a navigation resumed after a guard failed");
                            (parked.settled)(false);
                        }
                    }
                });

                if self.pending.borrow().is_some() {
                    self.questioned();
                }

                Ok(Navigation::Deferred)
            }
        }
    }

    /// Asks what stands in front of the destination whether it may be reached.
    ///
    /// Outermost first: an area's authorisation should refuse before an inner
    /// page considers anything, and the list arrives already in that order.
    ///
    /// Every guard on the route, not only the ones in front of segments about
    /// to be installed. Navigating between two pages of a guarded area keeps
    /// the area mounted, but "it let us in once" is not an answer to "may we
    /// be here now" - a session expires while the layout stays exactly where
    /// it was. Re-asking costs nothing: `Allow` allocates nothing.
    fn may_enter(&self, guards: &'static [&'static dyn EnterGuard], route: &str) -> Verdict {
        for guard in guards {
            let cx = EnterCx::new(self.host.services(), route);
            match guard.decide(&cx) {
                Verdict::Allow => {}
                refused => {
                    tracing::debug!(guard = guard.name(), route, "a guard refused a navigation");
                    return refused;
                }
            }
        }

        Verdict::Allow
    }

    /// Asks the scopes about to be torn down whether they may be.
    ///
    /// Innermost first: the leaf holds the unsaved form, and it should be the
    /// one that speaks. The scopes are cloned out before any guard runs - a
    /// guard is free to start another navigation, and would otherwise
    /// re-enter this borrow.
    fn may_leave(&self, shared_len: usize) -> Verdict {
        let leaving: Vec<Rc<Scope>> = match self.active.borrow().as_ref() {
            Some(active) => active
                .scopes
                .iter()
                .skip(kept_until(active.entries, shared_len))
                .cloned()
                .collect(),
            None => return Verdict::Allow,
        };

        for scope in leaving.iter().rev() {
            for guard in scope.leave_guards() {
                match guard() {
                    Verdict::Allow => {}
                    refused => return refused,
                }
            }
        }

        Verdict::Allow
    }

    /// The question a guard is waiting on, if there is one.
    ///
    /// Router state, not a call: a backend draws it like anything else it
    /// draws, and "a question is pending" and [`Navigation::Deferred`] are the
    /// same condition rather than two.
    ///
    /// A backend that draws over its own frame - a terminal, an immediate-mode
    /// one - **must swallow input while this is `Some`**, or the tabs keep
    /// switching underneath the dialog.
    pub fn pending(&self) -> Option<Ask> {
        self.pending.borrow().as_ref().map(|parked| parked.ask.clone())
    }

    /// Answers the pending question. Does nothing when there is none.
    pub fn answer(&self, allowed: bool) {
        let decision = self
            .pending
            .borrow()
            .as_ref()
            .map(|parked| parked.decision.clone());

        if let Some(decision) = decision {
            if allowed {
                decision.allow();
            } else {
                decision.block();
            }
        }
    }

    /// How much of the active chain the next one keeps: the segments that are
    /// the same type *and* captured the same thing.
    ///
    /// Shape alone is not enough. Two routes differing only in a parameter
    /// have identical chains, and every segment carrying that parameter - the
    /// leaf and every layout that derived it - has to install again with the
    /// new value.
    fn shared_len(&self, chain: &'static [SegmentEntry<U>], params: &[Box<dyn Any>]) -> usize {
        let active = self.active.borrow();
        let Some(active) = active.as_ref() else {
            return 0;
        };

        let by_shape = common_prefix_len(active.entries, chain);
        (0..by_shape)
            .take_while(|&index| match (active.params.get(index), params.get(index)) {
                (Some(before), Some(now)) => (chain[index].same_params)(&**before, &**now),
                _ => false,
            })
            .count()
    }

    /// Installs a navigation's chain, and says the router is at its route only
    /// once it is.
    fn commit(
        &self,
        chain: &'static [SegmentEntry<U>],
        shared_len: usize,
        params: Vec<Box<dyn Any>>,
        arrival: Arrival,
    ) -> anyhow::Result<Rc<Scope>> {
        let Arrival {
            route,
            described,
            again,
        } = arrival;

        match self.install_from(chain, shared_len, params, Some(again)) {
            Ok(leaf) => {
                *self.prev_route.borrow_mut() = Some(route);
                *self.described.borrow_mut() = Some(described);
                Ok(leaf)
            }
            Err(error) => {
                if self.active.borrow().is_none() {
                    *self.prev_route.borrow_mut() = None;
                    *self.described.borrow_mut() = None;
                }
                Err(error)
            }
        }
    }

    /// Replaces the active chain from `shared_len` down with `chain`.
    ///
    /// When a segment fails to install, what this navigation built comes down
    /// innermost first, and the chain it replaced goes back up - made again
    /// from its route's params, with the states its cached segments had. Only
    /// a chain that cannot be put back either, or one that came with no route,
    /// leaves the router with none.
    fn install_from(
        &self,
        chain: &'static [SegmentEntry<U>],
        shared_len: usize,
        params: Vec<Box<dyn Any>>,
        again: Option<Again>,
    ) -> anyhow::Result<Rc<Scope>> {
        // Taken before anything is dropped, so the states of cache-eligible
        // segments can be snapshotted along with what they captured.
        let prev = self.active.borrow_mut().take();

        let mut previous = None;
        let scopes: Vec<Rc<Scope>> = match prev {
            None => Vec::new(),
            Some(ActiveChain {
                entries,
                scopes,
                params: captured,
                again: before,
            }) => {
                previous = before.map(|before| (entries, before));
                let keeping = kept_until(entries, shared_len);
                let mut captured: Vec<Option<Box<dyn Any>>> =
                    captured.into_iter().map(Some).collect();
                {
                    let mut cache = self.state_cache.borrow_mut();
                    for (index, (entry, scope)) in
                        entries.iter().zip(scopes.iter()).enumerate().skip(keeping)
                    {
                        if entry.cache_state
                            && let Some(captured) = captured.get_mut(index).and_then(Option::take)
                        {
                            cache.insert(
                                cache_key(entries, index),
                                Cached {
                                    states: scope.snapshot_states(),
                                    params: captured,
                                },
                            );
                        }
                    }
                }

                self.leave(entries, scopes, captured, shared_len, keeping)
            }
        };

        let (error, kept) = match self.build(chain, scopes, &params) {
            Ok(scopes) => {
                let leaf = self.stand(chain, scopes, params, again);
                self.wake_standing();
                return Ok(leaf);
            }
            Err(failed) => failed,
        };

        let Some((entries, before)) = previous else {
            self.tear_down(kept, 0);
            return Err(error);
        };
        let captured = before();
        match self.build(entries, kept, &captured) {
            Ok(scopes) => {
                self.stand(entries, scopes, captured, Some(before));
                self.wake_standing();
                Err(error.context("the navigation failed; the router stayed where it was"))
            }
            Err((back, kept)) => {
                tracing::error!(error = %back, "the chain a failed navigation replaced would not go back");
                self.tear_down(kept, 0);
                Err(error)
            }
        }
    }

    /// Takes a chain down to its first `shared_len` segments, innermost
    /// first, and hands back what is left standing. The segments below that,
    /// up to `keeping`, go to sleep instead of going.
    fn leave(
        &self,
        entries: &[SegmentEntry<U>],
        scopes: Rc<Vec<Rc<Scope>>>,
        mut captured: Vec<Option<Box<dyn Any>>>,
        shared_len: usize,
        keeping: usize,
    ) -> Vec<Rc<Scope>> {
        let mut standing = match Rc::try_unwrap(scopes) {
            Ok(owned) => owned,
            Err(shared) => (*shared).clone(),
        };

        while standing.len() > shared_len {
            let scope = standing.pop().expect("longer than what stays");
            let index = standing.len();
            let params = (index < keeping)
                .then(|| captured.get_mut(index).and_then(Option::take))
                .flatten();

            let Some(params) = params else {
                self.forget_below(&scope);
                drop(scope);
                continue;
            };

            scope.sleep();
            let sleeping = Sleeping {
                scope,
                params,
                above: standing.iter().map(Rc::downgrade).collect(),
            };

            let replaced = self
                .sleeping
                .borrow_mut()
                .insert(cache_key(entries, index), sleeping);
            if let Some(replaced) = replaced {
                self.forget(replaced);
            }
        }

        standing
    }

    /// Tears `scopes` down to the first `shared_len`, with whatever sleeps
    /// below the ones that go.
    fn tear_down(&self, scopes: Vec<Rc<Scope>>, shared_len: usize) -> Vec<Rc<Scope>> {
        self.leave(&[], Rc::new(scopes), Vec::new(), shared_len, shared_len)
    }

    /// The segment that slept at `index` of `chain`, if it can wake there:
    /// under the very scopes it was installed under, with the same capture.
    /// One that cannot is torn down.
    fn wakeable(
        &self,
        chain: &'static [SegmentEntry<U>],
        index: usize,
        standing: &[Rc<Scope>],
        captured: &dyn Any,
    ) -> Option<Rc<Scope>> {
        let entry = &chain[index];
        let sleeping = self.sleeping.borrow_mut().remove(&cache_key(chain, index))?;

        if entry.keep && sleeping.sits_on(standing) && (entry.same_params)(&*sleeping.params, captured) {
            return Some(sleeping.scope);
        }

        self.forget(sleeping);
        None
    }

    /// Wakes what the active chain woke up with, outermost first, once the
    /// chain stands: a wake hook is free to navigate.
    fn wake_standing(&self) {
        let Some(scopes) = self.active_scopes() else {
            return;
        };

        for scope in scopes.iter().filter(|scope| !scope.is_awake()) {
            scope.wake();
        }
    }

    /// Tears down a sleeping segment, after whatever sleeps below it.
    fn forget(&self, sleeping: Sleeping) {
        self.forget_below(&sleeping.scope);
        drop(sleeping);
    }

    /// Tears down whatever sleeps below `scope`, deepest first.
    fn forget_below(&self, scope: &Rc<Scope>) {
        let mut below: Vec<Sleeping> = {
            let mut sleeping = self.sleeping.borrow_mut();
            let keys: Vec<StateCacheKey> = sleeping
                .iter()
                .filter(|(_, sleeping)| sleeping.is_below(scope))
                .map(|(key, _)| *key)
                .collect();

            keys.iter().filter_map(|key| sleeping.remove(key)).collect()
        };

        below.sort_by_key(|sleeping| std::cmp::Reverse(sleeping.above.len()));
        drop(below);
    }

    /// Tears down everything asleep, deepest first.
    fn forget_sleeping(&self) {
        let Ok(mut sleeping) = self.sleeping.try_borrow_mut() else {
            return;
        };
        let mut all: Vec<Sleeping> = sleeping.drain().map(|(_, sleeping)| sleeping).collect();
        drop(sleeping);

        all.sort_by_key(|sleeping| std::cmp::Reverse(sleeping.above.len()));
        drop(all);
    }

    /// Makes `chain` the active one, and hands back its leaf.
    fn stand(
        &self,
        chain: &'static [SegmentEntry<U>],
        scopes: Vec<Rc<Scope>>,
        params: Vec<Box<dyn Any>>,
        again: Option<Again>,
    ) -> Rc<Scope> {
        let leaf = scopes.last().expect("chain is non-empty").clone();
        *self.active.borrow_mut() = Some(ActiveChain {
            entries: chain,
            scopes: Rc::new(scopes),
            params,
            again,
        });
        leaf
    }

    /// Installs `chain` below the scopes already standing, one segment each.
    ///
    /// On a failure, what this built comes down the way it went up, innermost
    /// first - including the segment that failed, which may have installed
    /// half of itself - and the scopes that stood before are handed back.
    fn build(
        &self,
        chain: &'static [SegmentEntry<U>],
        mut scopes: Vec<Rc<Scope>>,
        params: &[Box<dyn Any>],
    ) -> Result<Vec<Rc<Scope>>, (anyhow::Error, Vec<Rc<Scope>>)> {
        let standing = scopes.len();

        for (index, entry) in chain.iter().enumerate().skip(standing) {
            let captured: &dyn Any = params
                .get(index)
                .map(|p| &**p)
                .expect("params has one entry per segment - the macro emits them together");

            if let Some(asleep) = self.wakeable(chain, index, &scopes, captured) {
                scopes.push(asleep);
                continue;
            }

            let scope = Rc::new(Scope::new());
            if entry.cache_state
                && let Some(cached) = self.state_cache.borrow_mut().take(cache_key(chain, index))
            {
                // Only back to the segment it came from. A page cached under
                // one set of parameters is a different page's worth of state
                // under another - restoring it is how metrics for one host
                // would come back showing another's.
                if (entry.same_params)(&*cached.params, captured) {
                    scope.restore_states(cached.states);
                }
            }

            // The ancestors snapshot is everything built so far this loop -
            // root to this segment's immediate parent, never including
            // `scope` itself. `inherit()` walks it to find an ancestor that
            // already `install()`-ed the feature being asked for.
            let ctx = self
                .host
                .context(scope.clone(), Rc::from(scopes.clone()));

            if let Err(error) = (entry.install)(&ctx, captured) {
                scopes.push(scope);
                let kept = self.tear_down(scopes, standing);

                return Err((error, kept));
            }

            scopes.push(scope);
        }

        Ok(scopes)
    }

    pub fn deactivate(&self) {
        // Through `unwind` rather than by dropping the chain, so that shutting
        // down takes the segments apart in the same direction a navigation
        // does. This is the path a closing window takes, which is the one
        // place where getting it wrong is hardest to notice.
        self.forget_sleeping();
        if let Some(active) = self.active.borrow_mut().take() {
            unwind(active.scopes, 0);
        }
        *self.prev_route.borrow_mut() = None;
    }
    
    /// The scope of the segment at `cursor` in the active chain: 0 is the
    /// outermost layout, the last one the leaf.
    pub fn scope_at(&self, cursor: usize) -> Option<Rc<Scope>> {
        self.active
            .borrow()
            .as_ref()
            .and_then(|active| active.scopes.get(cursor).cloned())
    }

    /// The route the router is on, if it is the type asked for.
    ///
    /// The router keeps it erased - it never learns `R` - so this is the way
    /// back to the typed route, and it is `None` before the first navigation.
    pub fn current_route<R: Clone + 'static>(&self) -> Option<R> {
        self.prev_route
            .borrow()
            .as_ref()
            .and_then(|route| (**route).downcast_ref::<R>())
            .cloned()
    }

    /// Records where the application is leaving from. Called by
    /// [`NavigateHandle::to`]; a forward stack only survives until the next
    /// ordinary navigation, exactly as a browser's does.
    pub fn remember(&self, route: Box<dyn Any>) {
        self.back.borrow_mut().push(Visited { route });
        self.forward.borrow_mut().clear();
    }

    /// Takes the previous entry, putting `leaving` on the forward stack.
    pub fn take_back(&self, leaving: Option<Box<dyn Any>>) -> Option<Visited> {
        let entry = self.back.borrow_mut().pop()?;
        if let Some(route) = leaving {
            self.forward.borrow_mut().push(Visited { route });
        }
        Some(entry)
    }

    /// Takes the next entry, putting `leaving` back on the history.
    pub fn take_forward(&self, leaving: Option<Box<dyn Any>>) -> Option<Visited> {
        let entry = self.forward.borrow_mut().pop()?;
        if let Some(route) = leaving {
            self.back.borrow_mut().push(Visited { route });
        }
        Some(entry)
    }

    /// Puts back a step a guard refused. The counterpart of
    /// [`take_back`](Self::take_back): the entry left the stack before anyone
    /// asked whether the move was allowed.
    pub fn restore_back(&self, route: Box<dyn Any>) {
        self.forward.borrow_mut().pop();
        self.back.borrow_mut().push(Visited { route });
    }

    pub fn restore_forward(&self, route: Box<dyn Any>) {
        self.back.borrow_mut().pop();
        self.forward.borrow_mut().push(Visited { route });
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.borrow().is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.borrow().is_empty()
    }

    /// The chain the active route installed, and the scopes it installed it
    /// into - what a backend needs to build props for a segment.
    pub fn active_chain(&self) -> Option<&'static [SegmentEntry<U>]> {
        self.active.borrow().as_ref().map(|active| active.entries)
    }

    pub fn active_scopes(&self) -> Option<Rc<Vec<Rc<Scope>>>> {
        self.active
            .borrow()
            .as_ref()
            .map(|active| active.scopes.clone())
    }

    pub fn active_scope(&self) -> Option<Rc<Scope>> {
        self.active.borrow().as_ref().and_then(|a| a.scopes.last().cloned())
    }
    
    /// The active chain, mounted.
    ///
    /// `nodes` is the backend's own store, borrowed for as long as the view
    /// lives - which is what lets a widget hold a reference into the state it
    /// shows instead of a copy of it.
    /// Says that a frame is being drawn from the active chain until the
    /// guard is dropped, so a navigation from inside the drawing waits for
    /// [`Router::settle`] instead of happening underneath it.
    ///
    /// For an immediate-mode backend, where drawing borrows the chain: the
    /// view holds the scopes alive, so tearing them down mid-frame drops
    /// nothing and leaves the teardown to run later, outside in, when the
    /// frame lets go. A retained backend builds its view and hands it over,
    /// and has nothing to hold.
    pub fn drawing(&self) -> Drawing<'_, U> {
        self.drawing.set(true);

        Drawing { router: self }
    }

    /// Carries out what was asked for during the frame, and says whether
    /// anything moved.
    pub fn settle(self: &Rc<Self>) -> anyhow::Result<bool> {
        let Some(held) = self.held.borrow_mut().take() else {
            return Ok(false);
        };

        match self.commit(held.chain, held.shared_len, held.params, held.arrival) {
            Ok(_) => {
                (held.settled)(true);
                Ok(true)
            }
            Err(error) => {
                (held.settled)(false);
                Err(error)
            }
        }
    }

    pub fn render<'a>(&self, nodes: &'a U::Nodes) -> U::View<'a> {
        self.active
            .borrow()
            .as_ref()
            .expect(
                "Router::render called with no active chain - either activate/navigate has not \
                 been called yet, or an install failed and took the chain it replaced with it",
            )
            .root_view(nodes)
    }
}
