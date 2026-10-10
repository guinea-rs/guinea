use std::fmt::Debug;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use crate::timers::{self, Period, Timer};
use anyhow::Context as _;
use guinea_core::SharedState;
use guinea_core::actor::shape::{Declared, name};
use guinea_core::actor::{Addr, Handler, Home, ManagedActor};
use guinea_core::trace::Bus;
use guinea_core::actor::event_bus::{EventBus, GlobalEventBus};
use guinea_core::actor::event_bus::subscribe::Event;
use guinea_core::feature::{Claim, Exported};
use guinea_core::guard::{Ask, Verdict};
use guinea_core::scope::{DropGuard, Reducer, Scope};

pub struct AppFeatureDeinitContext<'a> {
    pub shared: &'a SharedState,
}

/// What installing into a scope may do, wherever the scope is: a segment's in
/// a window, or the application's own.
///
/// A plugin and an application feature are handed it through their builder,
/// a segment's feature through [`FeatureInitContext`], and both say the same
/// things the same way: claim state and drive it, answer actions, watch other
/// state, spawn actors, run timers. What only a window has - its bus, its
/// place in a chain, leaving and waking - is the window context's own.
#[derive(Clone)]
pub struct ScopeContext {
    pub scope: Scope,
    /// What plugins provided during application startup.
    pub services: SharedState,
}

/// What a segment's feature is handed: the [`ScopeContext`] of its scope, and
/// the window it is in.
#[derive(Clone)]
pub struct FeatureInitContext {
    pub scope_cx: ScopeContext,
    /// Where in its chain the segment being installed sits: 0 for the
    /// outermost. The scopes above it are the tree's - `scope.ancestors()`.
    pub cursor: usize,
    /// Which root this feature is being installed into - the second window,
    /// or the only one. What a service shared between roots uses to tell
    /// callers apart.
    pub root: crate::app::roots::RootId,
    pub event_bus: Rc<EventBus>,
}

impl std::ops::Deref for FeatureInitContext {
    type Target = ScopeContext;

    fn deref(&self) -> &ScopeContext {
        &self.scope_cx
    }
}

/// A named unit with its own lifetime, its own state, and one bit saying it is
/// installed here.
///
/// The layer that joins the two halves: it claims reducers and gives them
/// something to drive them, and it is the only thing that may know about both.
/// What it publishes is [`Exports`](Feature::Exports) - everything else it
/// claims stays its own.
///
/// Written as a manifest and a function, the way an actor is:
///
/// ```ignore
/// feature! {
///     pub Processes {
///         exports { contracts::Processes }
///     }
/// }
///
/// #[installs]
/// fn processes(cx: &FeatureInitContext, context: &str) -> anyhow::Result<Processes> {
///     let (listing, _) = cx.state::<contracts::Processes>()
///         .driven_by(|push| ProcessActor::new(context.to_string(), push, cx.event_bus.clone()));
///
///     listing.emit(Refresh);
///     Ok(Processes(listing))
/// }
/// ```
///
/// The feature is what the function returns, built from what it exports - a
/// `Bound<R>` comes only from claiming `R`, so a feature cannot export what it
/// did not claim.
///
/// It is returned rather than dropped so that a segment installing two
/// features can wire them to each other - which is usually why it installs two.
pub trait Feature: Sized + 'static {
    /// What the segment hands it - typically what the route captured.
    type Params: ?Sized;

    /// The reducers segments below may read. `()` for a feature that publishes
    /// nothing, `(A,)` for one, `(A, B)` for two.
    type Exports: Exported;

    /// Where the feature was written. `#[installs]` fills it in; a feature
    /// implemented by hand leaves it unknown and loses only the source link.
    const DECLARED: Option<Declared> = None;

    fn install(cx: &FeatureInitContext, params: &Self::Params) -> anyhow::Result<Self>;
}

/// What `feature!` declares about a feature: what it exports. `#[installs]`
/// reads a feature's [`Exports`](Feature::Exports) from here.
pub trait Manifest {
    type Exports: Exported;
}

impl FeatureInitContext {
    /// Installs `F` here, and publishes what it exports.
    ///
    /// Twice for the same feature in one scope is a setup bug rather than
    /// something to merge silently, so it panics - the same rule as before,
    /// now with something real behind the bit.
    pub fn install<F: Feature>(&self, params: &F::Params) -> anyhow::Result<F> {
        self.scope.mark_feature_installed::<F>();
        F::Exports::mark(self.scope);

        // Its own corner of the scope, so that two instances of one feature
        // answering the same action type do not become one.
        self.scope.open_section(Some(name::<F>()), F::DECLARED);
        let installed = F::install(self, params);
        self.scope.close_section();

        // After the body, because only the body can claim anything. An export
        // the feature never claimed would otherwise be read below as the
        // reducer's `Default`, for as long as the application runs, and look
        // exactly like a feature that has not pushed an update yet.
        if installed.is_ok()
            && let Some(name) = F::Exports::unclaimed(self.scope)
        {
            panic!(
                "{} exports {name}, but nothing in it claimed that reducer - \
                 either claim it with `cx.state::<{name}>()`, or take it out of `Exports`",
                std::any::type_name::<F>()
            );
        }

        installed
    }

    /// Claims `R` for this scope, and says what else is true of it.
    ///
    /// The one way in. Ownership used to be a side effect of which of four
    /// calls a feature happened to make - `port`, `actions`, `wire`,
    /// `seed_reducer` - so a reducer could be claimed by accident and an actor
    /// could be left unwired without anything failing to build. Here the claim
    /// is the call, and continuing it is optional:
    ///
    /// ```ignore
    /// let (processes, actor) = cx.state::<Processes>()
    ///     .driven_by(|push| ProcessActor::new(context.to_string(), push, cx.event_bus.clone()));
    /// actor.subscribe_on::<ScanTick>(Bus::Global);
    ///
    /// processes.emit(Refresh);
    /// ```
    ///
    /// Ending it at [`plain`](Claim::plain) is not a half-written feature - it
    /// is state the UI owns, and `emit` on it does not compile.
    #[track_caller]
    pub fn state<R: Reducer>(&self) -> Claim<'_, R> {
        self.scope_cx.claim(Some(&self.event_bus), Location::caller())
    }

    /// Runs `hook` every time this segment wakes - for one declared `keep`
    /// in `routes!`, which the router puts to sleep instead of tearing down.
    ///
    /// What it slept through it missed: its timers did not tick and the
    /// buses did not tell it anything. This is where it catches up - asks for
    /// a fresh report, resets what counted the gap.
    pub fn on_wake(&self, hook: impl Fn() + 'static) {
        self.scope.on_wake(hook);
    }

    /// Asked before this segment is torn down by a navigation.
    ///
    /// Declared here, on the way in, because on the way out the scope exists
    /// and the guard can read its own state - which is what "unsaved changes"
    /// is. Entering is the asymmetric case: there is nothing to read yet, so
    /// an enter guard belongs in the route declaration instead.
    pub fn on_leave(&self, guard: impl Fn() -> Verdict + 'static) {
        self.scope.on_leave(guard);
    }

    /// Refuse to leave while `dirty` says so.
    ///
    /// No question and no dialog - for the case where leaving is simply not
    /// allowed yet. When the user should get a say, use
    /// [`confirm_leave`](Self::confirm_leave).
    pub fn block_leave_while<R: Reducer>(&self, dirty: impl Fn(&R) -> bool + 'static) {
        let state = self.scope.state::<R>();
        self.scope.on_leave(move || {
            if dirty(&state.borrow()) {
                Verdict::Block
            } else {
                Verdict::Allow
            }
        });
    }

    /// Ask before leaving while `dirty` says so.
    ///
    /// `question` is a closure rather than a value because it is built at the
    /// moment of asking: the language may have changed since install, and so
    /// may whatever the text names.
    pub fn confirm_leave<R: Reducer>(
        &self,
        dirty: impl Fn(&R) -> bool + 'static,
        question: impl Fn(&R) -> Ask + 'static,
    ) {
        let state = self.scope.state::<R>();
        self.scope.on_leave(move || {
            let state = state.borrow();
            if dirty(&state) {
                Verdict::ask(question(&state))
            } else {
                Verdict::Allow
            }
        });
    }

    /// Hears `M` on this window's bus, for as long as this segment lives and
    /// while it is awake.
    #[track_caller]
    pub fn subscribe<M: Event>(&self, callback: impl Fn(M) + 'static) {
        self.scope.note_listener(name::<M>(), None, Bus::Window);
        self.scope
            .own_subscription(self.event_bus.subscribe_fn(self.while_awake(callback)));
    }

    /// Creates `actor` in this segment, where it can hear this window's bus.
    /// See [`ScopeContext::spawn`].
    pub fn spawn<A: ManagedActor + Debug + 'static>(&self, actor: A) -> Addr<A> {
        self.scope_cx.spawn_in(Some(&self.event_bus), actor)
    }
}

impl ScopeContext {
    /// Claims `R` for this scope, and says what else is true of it. See
    /// [`FeatureInitContext::state`]; here the claim is the application's,
    /// and an actor driving it lives in no window.
    #[track_caller]
    pub fn state<R: Reducer>(&self) -> Claim<'_, R> {
        self.claim(None, Location::caller())
    }

    fn claim<'a, R: Reducer>(
        &'a self,
        bus: Option<&'a Rc<EventBus>>,
        at: &'static Location<'static>,
    ) -> Claim<'a, R> {
        self.scope.note_reducer_declared::<R>(Declared {
            file: at.file(),
            line: at.line(),
            column: at.column(),
            crate_dir: self.scope.current_crate_dir().unwrap_or_default(),
        });

        Claim::new(self.scope, bus)
    }

    /// Says this scope answers `M`, and how.
    ///
    /// The way in for a domain that does not use an actor - a task holding a
    /// `RefCell`, a channel, a plain closure. Nothing the UI touches can tell
    /// the difference, which is the point: how a domain implements its logic
    /// is its own business.
    ///
    /// `actor!` calls this for every handler it lists, so a feature with an
    /// actor never writes it by hand.
    pub fn answers<M: 'static>(
        &self,
        answer: impl Fn(M) + 'static,
    ) {
        self.scope.answers(answer);
    }

    /// A service a plugin provided at startup.
    ///
    /// The counterpart of `PluginBuilder::provide`: this is how a page reaches
    /// the store, or anything else an application-level plugin set up.
    pub fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        let service = std::any::type_name::<T>();
        match self.services.try_get::<T>() {
            Ok(Some(value)) => Ok(value),
            Ok(None) => anyhow::bail!(
                "no plugin provided service `{service}` - install the plugin that \
                 provides it on the application, before the window opens"
            ),
            Err(poisoned) => Err(anyhow::Error::new(poisoned))
                .with_context(|| format!("requiring service `{service}`")),
        }
    }

    pub fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services.get::<T>()
    }

    /// `R` as it is now, from this scope or the nearest one above that
    /// exports it; `None` when nothing in reach does. Read once: nothing here
    /// hears it change - [`observe`](Self::observe) does.
    pub fn read<R: Reducer>(&self) -> Option<Rc<R>> {
        let owner = self.scope.owner_of::<R>()?;
        Some(owner.binding::<R>().get())
    }

    /// What the application provided, or `T::default()` when it provided
    /// nothing: for settings with a sensible default that an application may
    /// override.
    pub fn require_or_default<T: Clone + Default + Send + Sync + 'static>(&self) -> T {
        self.try_require::<T>()
            .map_or_else(T::default, |provided| T::clone(&provided))
    }

    /// Reacts to what happens to `R`, wherever `R` lives.
    ///
    /// The coherence rule between two pieces of state that reference each
    /// other rather than nest: a cursor into a list the domain refreshes has
    /// to hear that the list was replaced, and a rename is not a replacement.
    /// The update itself is what tells those apart, so this is handed the
    /// update rather than told that something moved.
    ///
    /// Runs before anything is asked to redraw, and the subscription is owned
    /// by this scope - the rule dies with the scope that declared it.
    pub fn observe<R: Reducer>(&self, callback: impl Fn(&R::Update) + 'static) {
        let owner = self.scope.owner_of::<R>().unwrap_or_else(|| {
            panic!(
                "observing {} here found no scope that owns it: this segment did not \
                 claim it, and no ancestor exported it",
                std::any::type_name::<R>()
            )
        });

        let awake = self.scope.awake();
        self.scope.own(DropGuard(owner.observe::<R>(move |update| {
            if awake.now() {
                callback(update);
            }
        })));
    }

    /// Hears `M` on the global bus, for as long as this scope lives and while
    /// it is awake.
    #[track_caller]
    pub fn subscribe_global<M: Event>(&self, callback: impl Fn(M) + 'static) {
        self.scope.note_listener(name::<M>(), None, Bus::Global);
        self.scope
            .own(GlobalEventBus::subscribe_fn(self.while_awake(callback)));
    }

    pub(crate) fn while_awake<M>(&self, callback: impl Fn(M) + 'static) -> impl Fn(M) + 'static {
        let awake = self.scope.awake();
        move |event| {
            if awake.now() {
                callback(event);
            }
        }
    }

    /// Creates `actor`, subscribes it to what it listens for, and makes it
    /// this scope's: listed for devtools, and disposed when the scope goes.
    pub fn spawn<A: ManagedActor + Debug + 'static>(&self, actor: A) -> Addr<A> {
        self.spawn_in(None, actor)
    }

    fn spawn_in<A: ManagedActor + Debug + 'static>(
        &self,
        bus: Option<&Rc<EventBus>>,
        actor: A,
    ) -> Addr<A> {
        Addr::new_managed(actor, &Home::new(self.scope, bus))
    }

    /// Sends `message()` to `addr` every `period`, for as long as this scope
    /// lives.
    #[track_caller]
    pub fn every<A, M>(
        &self,
        period: impl Into<Period>,
        addr: &Addr<A>,
        message: impl Fn() -> M + 'static,
    ) -> Timer
    where
        A: Handler<M>,
        M: Send + 'static,
    {
        let addr = addr.clone();
        self.start_timer(Location::caller(), period.into(), move || addr.send(message()))
    }

    /// Runs `run` every `period`, for as long as this scope lives.
    #[track_caller]
    pub fn repeat(&self, period: impl Into<Period>, run: impl FnMut() + 'static) -> Timer {
        self.start_timer(Location::caller(), period.into(), run)
    }

    fn start_timer(
        &self,
        place: &'static Location<'static>,
        period: Period,
        run: impl FnMut() + 'static,
    ) -> Timer {
        let (ticking, timer) = timers::start(
            place,
            self.scope.current_feature(),
            Some((self.scope.key(), self.scope.awake())),
            period,
            run,
        );
        self.scope.own(ticking);

        timer
    }
}
