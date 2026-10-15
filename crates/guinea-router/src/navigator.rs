//! Where a window is, as the application's own route type.
//!
//! A route is a plain enum, nested one level per layout:
//! `Route::Processes(ProcessesRoute)`. The navigator holds where the window
//! is at the outermost level, with its history, in the scope it
//! was opened in; a layout lower down asks for its own level -
//! `Navigator::<ProcessesRoute>::find` - and gets the same navigator seen from
//! there, never naming the enum around it.
//!
//! Nothing here installs or draws. A layout's outlet mounts what the route
//! says and tells the navigator what it rendered, which is how the navigator
//! knows whom to ask before leaving: only an outlet whose level changes - not
//! one whose variant [`stays`](Routable::stays) - has its child asked.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use guinea_core::guard::{Ask, Decision, Verdict};
use guinea_core::scope::Scope;

use guinea_core::observability::observer::{self, Event, Navigation};

/// A place the application can be: an enum, a variant per place, and a
/// variant holding another route for each layout below.
///
/// `Debug` is how devtools write where a window is.
pub trait Routable: Clone + PartialEq + std::fmt::Debug + 'static {
    /// The variant, for a log line and for devtools.
    fn name(&self) -> &'static str;

    /// Whether moving from here to `to` keeps what this level mounted: the
    /// same variant holding a nested route, where only the level below
    /// changes.
    fn stays(&self, to: &Self) -> bool {
        self == to
    }

    /// Every route type nested in this one, at any depth.
    fn levels() -> Vec<Level<Self>> {
        Vec::new()
    }

    /// Every place, each with what its fields default to - `Err` with the
    /// variant's name for one whose fields have no default.
    fn samples() -> Vec<Result<Self, &'static str>> {
        Vec::new()
    }

    /// Every address this route type answers to, nested levels included.
    fn deep_links() -> Vec<crate::link::DeepLink> {
        Vec::new()
    }

    /// The address this route answers to, when every level of it has one.
    fn link(&self) -> Option<String> {
        let mut segments = Vec::new();
        self.write(&mut segments)
            .then(|| format!("/{}", segments.join("/")))
    }

    /// The route at `link`, when one answers to it.
    ///
    /// The leading slash is optional and one trailing slash is forgiven; any
    /// other empty segment is a segment, so an empty capture reads back as
    /// itself and `//` is not skipped.
    fn parse(link: &str) -> Option<Self> {
        let path = link.strip_prefix('/').unwrap_or(link);
        let segments: Vec<&str> = match path.is_empty() {
            true => Vec::new(),
            false => path.split('/').collect(),
        };

        Self::read(&segments)
            .or_else(|| match segments.split_last() {
                Some((&"", rest)) => Self::read(rest),
                _ => None,
            })
            .map(|(route, _)| route)
    }

    /// Writes this route's path segments after the ones already in `into`;
    /// `false` when a level of it has no link.
    #[doc(hidden)]
    fn write(&self, _into: &mut Vec<String>) -> bool {
        false
    }

    /// The route the whole of `segments` reaches, and which of them it read
    /// as captures: of two that reach it, the one with a literal first wins.
    #[doc(hidden)]
    fn read(_segments: &[&str]) -> Option<(Self, Vec<bool>)> {
        None
    }
}

/// A route type nested in `R`, and how to get from one to the other.
pub struct Level<R> {
    of: TypeId,
    project: Rc<dyn Fn(&R) -> Option<&dyn Any>>,
    wrap: Rc<dyn Fn(Box<dyn Any>) -> R>,
    stays: fn(&dyn Any, &dyn Any) -> bool,
}

impl<R: Routable> Level<R> {
    /// `T` nested in `R` through one variant, and everything nested in `T`.
    pub fn nested<T: Routable>(project: fn(&R) -> Option<&T>, wrap: fn(T) -> R) -> Vec<Level<R>> {
        let mut levels = vec![Level {
            of: TypeId::of::<T>(),
            project: Rc::new(move |route: &R| project(route).map(|nested| nested as &dyn Any)),
            wrap: Rc::new(move |nested: Box<dyn Any>| wrap(*nested.downcast::<T>().expect("wrapped at its own level"))),
            stays: stays::<T>,
        }];

        for inner in T::levels() {
            let (inward, outward) = (inner.project.clone(), inner.wrap.clone());
            levels.push(Level {
                of: inner.of,
                project: Rc::new(move |route: &R| project(route).and_then(|nested| inward(nested))),
                wrap: Rc::new(move |nested: Box<dyn Any>| wrap(outward(nested))),
                stays: inner.stays,
            });
        }

        levels
    }
}

fn stays<T: Routable>(from: &dyn Any, to: &dyn Any) -> bool {
    match (from.downcast_ref::<T>(), to.downcast_ref::<T>()) {
        (Some(from), Some(to)) => from.stays(to),
        _ => false,
    }
}

/// What became of a navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// It is there; what it mounts is installed when it is drawn.
    Done,
    /// A guard asked a question: nothing has changed until it is answered.
    /// See [`Navigator::pending`].
    Deferred,
    /// A guard refused.
    Blocked,
}

/// Where a window is, at the level of `T`.
///
/// Opened once, in the scope that holds it, and found from anywhere under
/// that scope: at the route type it was opened with, or at any route type
/// nested in it.
pub struct Navigator<T> {
    root: Rc<dyn Navigates>,
    route: PhantomData<T>,
}

impl<T> Clone for Navigator<T> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            route: PhantomData,
        }
    }
}

impl<T> PartialEq for Navigator<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.root, &other.root)
    }
}

/// Hears a navigator move until dropped.
#[must_use = "the watch ends when this is dropped"]
pub struct Watching {
    root: Weak<dyn Navigates>,
    id: u64,
}

impl Drop for Watching {
    fn drop(&mut self) {
        if let Some(root) = self.root.upgrade() {
            root.unwatch(self.id);
        }
    }
}

/// The navigators a scope holds, one per route type it was opened with.
#[derive(Default)]
struct Held(Vec<Rc<dyn Navigates>>);

impl<T: Routable> Navigator<T> {
    /// A navigator over `T` held by `scope`, starting at `start` - or the one
    /// `scope` already holds, which keeps where it is.
    pub fn open(scope: Scope, start: T) -> Self {
        if let Some(open) = held(scope, TypeId::of::<T>()) {
            return Self::over(open);
        }

        let root: Rc<dyn Navigates> = Rc::new(Root {
            window: scope,
            levels: T::levels(),
            state: RefCell::new(State {
                current: start,
                back: Vec::new(),
                forward: Vec::new(),
                outlets: Vec::new(),
                pending: None,
                undo: None,
                failure: None,
            }),
            watchers: RefCell::new(Vec::new()),
            next_watcher: Cell::new(0),
            generation: Cell::new(0),
        });

        if !scope.keeps::<Held>() {
            scope.keep(Held::default());
        }
        scope.kept::<Held, _>(|held| held.0.push(root.clone()));
        observer::tell(scope, || Event::Navigated {
            scope: scope.id(),
            navigation: root.navigation(),
        });

        Self::over(root)
    }

    /// The navigator that serves `T` nearest above `from`, `from` itself
    /// included.
    pub fn find(from: Scope) -> Option<Self> {
        let level = TypeId::of::<T>();
        std::iter::successors(Some(from), Scope::parent)
            .find_map(|scope| held(scope, level))
            .map(Self::over)
    }

    fn over(root: Rc<dyn Navigates>) -> Self {
        Self {
            root,
            route: PhantomData,
        }
    }

    /// Where the window is at this level; `None` while it is somewhere this
    /// level is not part of.
    pub fn current(&self) -> Option<T> {
        let mut found = None;
        self.root.current_at(TypeId::of::<T>(), &mut |at| {
            found = at.and_then(|at| at.downcast_ref::<T>()).cloned();
        });
        found
    }

    /// Goes to `route`, remembering where it was.
    pub fn to(&self, route: T) -> Moved {
        self.root.clone().go(TypeId::of::<T>(), Box::new(route))
    }

    /// A click handler that goes to `route` each time it is called.
    pub fn to_handler(&self, route: T) -> impl Fn() + Clone + 'static {
        let navigator = self.clone();
        move || {
            navigator.to(route.clone());
        }
    }

    /// One step back, if there is one; whether it moved now.
    pub fn back(&self) -> bool {
        self.root.clone().step(Step::Back)
    }

    /// One step forward, undoing a [`back`](Self::back).
    pub fn forward(&self) -> bool {
        self.root.clone().step(Step::Forward)
    }

    pub fn can_go_back(&self) -> bool {
        self.root.can_step(Step::Back)
    }

    pub fn can_go_forward(&self) -> bool {
        self.root.can_step(Step::Forward)
    }

    /// The question a guard is waiting on, if there is one.
    pub fn pending(&self) -> Option<Ask> {
        self.root.pending()
    }

    /// Answers the pending question; nothing when there is none.
    pub fn answer(&self, allowed: bool) {
        self.root.answer(allowed);
    }

    /// Says that `owner`'s outlet at this level drew `child` for `at`: what
    /// is asked before a navigation takes it away.
    pub fn rendered(&self, owner: Scope, child: Scope, at: &T) {
        self.root
            .rendered(TypeId::of::<T>(), owner, child, Box::new(at.clone()));
    }

    /// What `owner`'s outlet at this level drew last, while it stands.
    pub fn drawn(&self, owner: Scope) -> Option<Scope> {
        self.root.drawn(TypeId::of::<T>(), owner)
    }

    /// Says that what the window moved to could not be mounted: it goes back
    /// to where it was, and keeps why for [`failure`](Self::failure).
    pub fn failed(&self, error: anyhow::Error) {
        self.root.failed(error);
    }

    /// Why the last place it moved to could not be mounted, until it next
    /// moves.
    pub fn failure(&self) -> Option<String> {
        self.root.failure()
    }

    /// Runs `heard` after every move, and whenever a guard's question comes
    /// or goes, until the returned watch is dropped.
    pub fn watch(&self, heard: impl Fn() + 'static) -> Watching {
        let id = self.root.watch(Rc::new(heard));
        Watching {
            root: Rc::downgrade(&self.root),
            id,
        }
    }
}

/// A navigator as a window's question dialog sees it, whatever it navigates.
pub trait Asking {
    /// See [`Navigator::pending`].
    fn pending(&self) -> Option<Ask>;
    /// See [`Navigator::answer`].
    fn answer(&self, allowed: bool);
    /// See [`Navigator::watch`].
    fn watch(&self, heard: Box<dyn Fn()>) -> Watching;
}

impl<T: Routable> Asking for Navigator<T> {
    fn pending(&self) -> Option<Ask> {
        Navigator::pending(self)
    }

    fn answer(&self, allowed: bool) {
        Navigator::answer(self, allowed);
    }

    fn watch(&self, heard: Box<dyn Fn()>) -> Watching {
        Navigator::watch(self, heard)
    }
}

fn held(scope: Scope, level: TypeId) -> Option<Rc<dyn Navigates>> {
    if !scope.keeps::<Held>() {
        return None;
    }
    scope
        .kept::<Held, _>(|held| held.0.iter().find(|root| root.serves(level)).cloned())
        .flatten()
}

#[derive(Clone, Copy)]
enum Step {
    To,
    Back,
    Forward,
}

trait Navigates {
    fn serves(&self, level: TypeId) -> bool;
    fn current_at(&self, level: TypeId, with: &mut dyn FnMut(Option<&dyn Any>));
    fn go(self: Rc<Self>, level: TypeId, to: Box<dyn Any>) -> Moved;
    fn step(self: Rc<Self>, step: Step) -> bool;
    fn can_step(&self, step: Step) -> bool;
    fn pending(&self) -> Option<Ask>;
    fn answer(&self, allowed: bool);
    fn rendered(&self, level: TypeId, owner: Scope, child: Scope, at: Box<dyn Any>);
    fn drawn(&self, level: TypeId, owner: Scope) -> Option<Scope>;
    fn failed(&self, error: anyhow::Error);
    fn failure(&self) -> Option<String>;
    fn watch(&self, heard: Rc<dyn Fn()>) -> u64;
    fn unwatch(&self, id: u64);
    fn navigation(&self) -> Navigation;
}

/// What a tool that starts watching is told of the navigators `scope`
/// opened.
pub(crate) fn describe(scope: Scope, events: &mut Vec<Event>) {
    if !scope.keeps::<Held>() {
        return;
    }
    let navigations = scope
        .kept::<Held, _>(|held| held.0.iter().map(|root| root.navigation()).collect::<Vec<_>>())
        .unwrap_or_default();
    events.extend(navigations.into_iter().map(|navigation| Event::Navigated {
        scope: scope.id(),
        navigation,
    }));
}

struct Root<R: Routable> {
    window: Scope,
    levels: Vec<Level<R>>,
    state: RefCell<State<R>>,
    watchers: RefCell<Vec<(u64, Rc<dyn Fn()>)>>,
    next_watcher: Cell<u64>,
    generation: Cell<u64>,
}

struct State<R> {
    current: R,
    back: Vec<R>,
    forward: Vec<R>,
    outlets: Vec<Outlet>,
    pending: Option<Parked<R>>,
    undo: Option<Undo<R>>,
    failure: Option<String>,
}

/// What an outlet drew last, at its level.
struct Outlet {
    level: TypeId,
    owner: Scope,
    child: Scope,
    at: Box<dyn Any>,
}

struct Parked<R> {
    ask: Ask,
    decision: Decision,
    to: R,
    step: Step,
}

/// Where it was before the last move, for a move whose destination fails to
/// mount.
struct Undo<R> {
    current: R,
    back: Vec<R>,
    forward: Vec<R>,
}

impl<R: Routable> Root<R> {
    fn project<'r>(&self, level: TypeId, route: &'r R) -> Option<&'r dyn Any> {
        if level == TypeId::of::<R>() {
            return Some(route);
        }
        let found = self.levels.iter().find(|nested| nested.of == level)?;
        (found.project)(route)
    }

    fn wrap(&self, level: TypeId, route: Box<dyn Any>) -> Option<R> {
        if level == TypeId::of::<R>() {
            return route.downcast::<R>().ok().map(|route| *route);
        }
        let found = self.levels.iter().find(|nested| nested.of == level)?;
        Some((found.wrap)(route))
    }

    fn stays(&self, level: TypeId, from: &dyn Any, to: &dyn Any) -> bool {
        if level == TypeId::of::<R>() {
            return stays::<R>(from, to);
        }
        self.levels
            .iter()
            .find(|nested| nested.of == level)
            .is_some_and(|nested| (nested.stays)(from, to))
    }

    fn target(&self, step: Step) -> Option<R> {
        let state = self.state.borrow();
        match step {
            Step::To => None,
            Step::Back => state.back.last().cloned(),
            Step::Forward => state.forward.last().cloned(),
        }
    }

    fn navigate(self: Rc<Self>, to: R, step: Step) -> Moved {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let dropped = self.supersede();

        let _navigating = guinea_core::trace::enter(|| guinea_core::trace::Point::Navigate {
            root: guinea_app::app::roots::label(self.window).unwrap_or_default(),
            to: to.name().to_string(),
        });

        match self.may_leave(&to) {
            Verdict::Allow => {
                self.commit(to, step);
                Moved::Done
            }
            Verdict::Block => {
                if dropped {
                    self.moved();
                }
                Moved::Blocked
            }
            Verdict::Ask(ask, decision) => {
                self.state.borrow_mut().pending = Some(Parked {
                    ask,
                    decision: decision.clone(),
                    to,
                    step,
                });

                let root = Rc::downgrade(&self);
                decision.on_answer(move |allowed| {
                    let Some(root) = root.upgrade() else {
                        return;
                    };
                    if root.generation.get() != generation {
                        return;
                    }
                    let parked = root.state.borrow_mut().pending.take();
                    match (parked, allowed) {
                        (Some(parked), true) => root.commit(parked.to, parked.step),
                        (Some(_), false) => root.moved(),
                        (None, _) => {}
                    }
                });

                let parked = self.state.borrow().pending.is_some();
                match parked {
                    true => {
                        self.moved();
                        Moved::Deferred
                    }
                    false => Moved::Done,
                }
            }
        }
    }

    fn supersede(&self) -> bool {
        self.state.borrow_mut().pending.take().is_some()
    }

    fn may_leave(&self, to: &R) -> Verdict {
        let leaving = {
            let mut state = self.state.borrow_mut();
            state.outlets.retain(|outlet| outlet.owner.is_alive() && outlet.child.is_alive());

            let mut leaving: Vec<Scope> = Vec::new();
            for outlet in &state.outlets {
                let Some(next) = self.project(outlet.level, to) else {
                    continue;
                };
                if self.stays(outlet.level, &*outlet.at, next) {
                    continue;
                }
                for scope in outlet.child.subtree() {
                    if !leaving.contains(&scope) {
                        leaving.push(scope);
                    }
                }
            }
            leaving
        };

        for scope in leaving {
            for guard in scope.leave_guards() {
                match guard() {
                    Verdict::Allow => {}
                    refused => return refused,
                }
            }
        }
        Verdict::Allow
    }

    fn commit(&self, to: R, step: Step) {
        {
            let mut state = self.state.borrow_mut();
            state.undo = Some(Undo {
                current: state.current.clone(),
                back: state.back.clone(),
                forward: state.forward.clone(),
            });
            state.failure = None;

            let left = std::mem::replace(&mut state.current, to);
            match step {
                Step::To => {
                    state.back.push(left);
                    state.forward.clear();
                }
                Step::Back => {
                    state.back.pop();
                    state.forward.push(left);
                }
                Step::Forward => {
                    state.forward.pop();
                    state.back.push(left);
                }
            }
        }

        self.moved();
    }

    fn moved(&self) {
        observer::tell(self.window, || Event::Navigated {
            scope: self.window.id(),
            navigation: self.navigation(),
        });

        let watchers: Vec<Rc<dyn Fn()>> = self
            .watchers
            .borrow()
            .iter()
            .map(|(_, heard)| heard.clone())
            .collect();
        for heard in watchers {
            heard();
        }
    }
}

impl<R: Routable> Navigates for Root<R> {
    fn serves(&self, level: TypeId) -> bool {
        level == TypeId::of::<R>() || self.levels.iter().any(|nested| nested.of == level)
    }

    fn current_at(&self, level: TypeId, with: &mut dyn FnMut(Option<&dyn Any>)) {
        let state = self.state.borrow();
        with(self.project(level, &state.current));
    }

    fn go(self: Rc<Self>, level: TypeId, to: Box<dyn Any>) -> Moved {
        let Some(to) = self.wrap(level, to) else {
            return Moved::Blocked;
        };
        self.navigate(to, Step::To)
    }

    fn step(self: Rc<Self>, step: Step) -> bool {
        let Some(to) = self.target(step) else {
            return false;
        };
        self.navigate(to, step) == Moved::Done
    }

    fn can_step(&self, step: Step) -> bool {
        self.target(step).is_some()
    }

    fn pending(&self) -> Option<Ask> {
        self.state.borrow().pending.as_ref().map(|parked| parked.ask.clone())
    }

    fn answer(&self, allowed: bool) {
        let decision = self
            .state
            .borrow()
            .pending
            .as_ref()
            .map(|parked| parked.decision.clone());
        match (decision, allowed) {
            (Some(decision), true) => decision.allow(),
            (Some(decision), false) => decision.block(),
            (None, _) => {}
        }
    }

    fn rendered(&self, level: TypeId, owner: Scope, child: Scope, at: Box<dyn Any>) {
        let mut state = self.state.borrow_mut();
        let outlet = Outlet {
            level,
            owner,
            child,
            at,
        };
        match state
            .outlets
            .iter_mut()
            .find(|drawn| drawn.owner == owner && drawn.level == level)
        {
            Some(drawn) => *drawn = outlet,
            None => state.outlets.push(outlet),
        }
    }

    fn drawn(&self, level: TypeId, owner: Scope) -> Option<Scope> {
        self.state
            .borrow()
            .outlets
            .iter()
            .find(|drawn| drawn.owner == owner && drawn.level == level && drawn.child.is_alive())
            .map(|drawn| drawn.child)
    }

    fn failed(&self, error: anyhow::Error) {
        tracing::error!(error = format!("{error:#}"), "what the window moved to could not be mounted");
        {
            let mut state = self.state.borrow_mut();
            if let Some(undo) = state.undo.take() {
                state.current = undo.current;
                state.back = undo.back;
                state.forward = undo.forward;
            }
            state.failure = Some(format!("{error:#}"));
        }
        self.moved();
    }

    fn failure(&self) -> Option<String> {
        self.state.borrow().failure.clone()
    }

    fn watch(&self, heard: Rc<dyn Fn()>) -> u64 {
        let id = self.next_watcher.get();
        self.next_watcher.set(id + 1);
        self.watchers.borrow_mut().push((id, heard));
        id
    }

    fn unwatch(&self, id: u64) {
        self.watchers.borrow_mut().retain(|(watching, _)| *watching != id);
    }

    fn navigation(&self) -> Navigation {
        let state = self.state.borrow();
        Navigation {
            routes: std::any::type_name::<R>(),
            route: format!("{:?}", state.current),
            back: state.back.len(),
            forward: state.forward.len(),
            pending: state.pending.as_ref().map(|parked| parked.ask.text.clone()),
            failure: state.failure.clone(),
        }
    }
}
