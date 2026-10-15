use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use guinea_core::actor::Stream;
use guinea_core::sources::{Period, Source};
use guinea_core::actor::shape::{Declared, name};
use guinea_core::actor::{Addr, Home, ManagedActor, Receives};
use guinea_core::actor::event_bus::subscribe::Event;
use guinea_core::feature::{Claim, Exported};
use guinea_core::feed::{Feed, FeedWriter};
use guinea_core::guard::{Ask, Verdict};
use guinea_core::scope::{Reducer, Scope, Section};

use super::given::Lookup;

/// What installing into a scope may do, wherever the scope is: a segment's in
/// a window, or the application's own.
///
/// A plugin and an application feature are handed it through their builder,
/// a segment's feature through [`FeatureInitContext`], and both say the same
/// things the same way: claim state and drive it, answer actions, watch other
/// state, hear the bus, spawn actors, run timers. What only a window has - its
/// place in a chain, leaving and waking - is the window context's own.
#[derive(Clone)]
pub struct ScopeContext {
    pub(crate) scope: Scope,
    /// Whose what is registered through this context is: the feature or
    /// plugin installing, or the scope's own. A section of `scope`.
    section: Section,
}

impl ScopeContext {
    /// A context for `scope`'s own section.
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            section: scope.section(),
        }
    }

    /// This context for `section`, and the scope it is in.
    pub(crate) fn in_section(&self, section: Section) -> Self {
        Self {
            scope: section.scope(),
            section,
        }
    }

    /// The scope itself, for the adapters and the router. A feature reaches
    /// its scope only through what this context offers.
    #[doc(hidden)]
    pub fn __scope(&self) -> Scope {
        self.scope
    }
}

/// What a segment's feature is handed: the [`ScopeContext`] of its scope, and
/// the window it is in.
#[derive(Clone)]
pub struct FeatureInitContext {
    pub scope_cx: ScopeContext,
    /// Where in its chain the segment being installed sits: 0 for the
    /// outermost. The scopes above it are the tree's - `scope.ancestors()`.
    pub cursor: usize,
}

impl std::ops::Deref for FeatureInitContext {
    type Target = ScopeContext;

    fn deref(&self) -> &ScopeContext {
        &self.scope_cx
    }
}

/// What every install does once its body ran, in a window or in the
/// application: refuses `U` when it exports a reducer nothing in it claimed,
/// which a page below would otherwise read as the reducer's `Default` for as
/// long as the application runs; otherwise marks it installed and publishes
/// its exports.
pub(crate) fn settle<U: 'static, E: Exported>(scope: Scope, who: &'static str) -> anyhow::Result<()> {
    publish::<E>(scope, who)?;
    scope.mark_feature_installed::<U>();
    Ok(())
}

/// [`settle`] without marking anything installed: for a plugin, of which an
/// application may have several copies.
pub(crate) fn publish<E: Exported>(scope: Scope, who: &'static str) -> anyhow::Result<()> {
    if let Some(reducer) = E::unclaimed(scope) {
        anyhow::bail!(
            "`{who}` exports {reducer}, but nothing in it claimed that reducer - \
             claim it with `state::<{reducer}>()`, or take it out of `Exports`"
        );
    }

    E::mark(scope);
    Ok(())
}

impl super::given::Context for ScopeContext {
    fn lookup(&self) -> Lookup {
        Lookup(self.scope)
    }
}

impl super::given::Context for FeatureInitContext {
    fn lookup(&self) -> Lookup {
        Lookup(self.scope)
    }
}

impl<P: ?Sized> super::given::Context for super::Install<'_, P> {
    fn lookup(&self) -> Lookup {
        Lookup(self.scope)
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
///         .driven_by(|push| ProcessActor::new(context.to_string(), push));
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
/// What it returns is handles and nothing else: its state, actors, timers and
/// feeds are its scope's, and live as long as the scope does whether the
/// value is kept or not. It is returned so that a feature installing another
/// can wire the two together; a page or a layout drops it.
///
/// Written by `#[installs]`, not by hand.
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

/// What `feature!` or `plugin!` declares: what it exports. `#[installs]` and
/// `#[builds]` read [`Exports`](Feature::Exports) from here.
pub trait Manifest {
    type Exports: Exported;
}

impl FeatureInitContext {
    /// Installs `F` here, and publishes what it exports.
    ///
    /// Twice for the same feature in one scope is a setup bug rather than
    /// something to merge silently, so it panics, before the body runs.
    ///
    /// An install that fails publishes nothing, is not listed, and claims and
    /// answers nothing here - so `F` may be installed again once what it
    /// needed is there.
    ///
    /// The feature lives as long as this scope, and the value returned is
    /// only its handles: a page drops it, a feature installing another may
    /// wire the two with it.
    pub fn install<F: Feature>(&self, params: &F::Params) -> anyhow::Result<F> {
        let who = name::<F>();
        if self.scope.has_feature::<F>() {
            panic!("`{who}` is already installed in this scope - a scope installs a feature once");
        }

        let section = self.scope.open_section(Some(who), F::DECLARED);
        let cx = FeatureInitContext {
            scope_cx: self.scope_cx.in_section(section),
            ..self.clone()
        };

        let installed = F::install(&cx, params)
            .and_then(|installed| settle::<F, F::Exports>(self.scope, who).map(|()| installed));
        if installed.is_err() {
            section.withdraw();
        }
        installed
    }

    /// Runs `hook` every time this scope wakes - for one mounted with `keep`,
    /// which is put to sleep instead of let go when it is left out.
    ///
    /// What it slept through it missed: its timers did not tick and the
    /// buses did not tell it anything. This is where it catches up - asks for
    /// a fresh report, resets what counted the gap.
    pub fn on_wake(&self, hook: impl Fn() + 'static) {
        self.scope.on_wake(hook);
    }

    /// Runs `hook` once, when this scope goes - after what it holds is let
    /// go of in reverse, at the point it was declared.
    pub fn on_teardown(&self, hook: impl FnOnce() + 'static) {
        self.scope.on_teardown(hook);
    }

    /// Asked before this segment is torn down by a navigation.
    ///
    /// Declared here, on the way in, because on the way out the scope exists
    /// and the guard can read its own state - which is what "unsaved changes"
    /// is.
    pub fn on_leave(&self, guard: impl Fn() -> Verdict + 'static) {
        self.scope.on_leave(guard);
    }

    /// Refuse to leave while `dirty` says so.
    ///
    /// No question and no dialog - for the case where leaving is simply not
    /// allowed yet. When the user should get a say, use
    /// [`confirm_leave`](Self::confirm_leave).
    ///
    /// `R` is read when leaving, where a page here reads it: from this
    /// segment if it claimed `R`, or from the nearest one above that exports
    /// it. Declaring the guard claims and creates nothing, so it may come
    /// before the claim it reads. When nothing in reach holds `R`, there is
    /// nothing unsaved, and leaving is allowed.
    pub fn block_leave_while<R: Reducer>(&self, dirty: impl Fn(&R) -> bool + 'static) {
        let reach = self.scope_cx.clone();
        self.scope.on_leave(move || match reach.read::<R>() {
            Some(state) if dirty(&state) => Verdict::Block,
            _ => Verdict::Allow,
        });
    }

    /// Ask before leaving while `dirty` says so.
    ///
    /// `question` is a closure rather than a value because it is built at the
    /// moment of asking: the language may have changed since install, and so
    /// may whatever the text names.
    ///
    /// `R` is read as [`block_leave_while`](Self::block_leave_while) reads it.
    pub fn confirm_leave<R: Reducer>(
        &self,
        dirty: impl Fn(&R) -> bool + 'static,
        question: impl Fn(&R) -> Ask + 'static,
    ) {
        let reach = self.scope_cx.clone();
        self.scope.on_leave(move || match reach.read::<R>() {
            Some(state) if dirty(&state) => Verdict::ask(question(&state)),
            _ => Verdict::Allow,
        });
    }

}

impl ScopeContext {
    /// Claims `R` for this scope, and says what else is true of it.
    ///
    /// The one way in. Ownership used to be a side effect of which of four
    /// calls a feature happened to make - `port`, `actions`, `wire`,
    /// `seed_reducer` - so a reducer could be claimed by accident and an actor
    /// could be left unwired without anything failing to build. Here the claim
    /// is the call, and continuing it is optional:
    ///
    /// ```ignore
    /// let (processes, _) = cx.state::<Processes>()
    ///     .driven_by(|push| ProcessActor::new(context.to_string(), push));
    ///
    /// processes.emit(Refresh);
    /// ```
    ///
    /// Ending it at [`plain`](Claim::plain) is not a half-written feature - it
    /// is state the UI owns, and `emit` on it does not compile.
    #[track_caller]
    pub fn state<R: Reducer>(&self) -> Claim<R> {
        let at = Location::caller();
        let declared = Declared {
            file: at.file(),
            line: at.line(),
            column: at.column(),
            crate_dir: self
                .section
                .declared()
                .map(|declared| declared.crate_dir)
                .unwrap_or_default(),
        };

        Claim::new(self.section, declared)
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
        self.section.answers(answer);
    }

    /// `R` as it is now, from this scope or the nearest one above that
    /// exports it; `None` when nothing in reach does. Read once: nothing here
    /// hears it change - [`observe`](Self::observe) does.
    pub fn read<R: Reducer>(&self) -> Option<Rc<R>> {
        let owner = self.scope.owner_of::<R>()?;
        Some(owner.binding::<R>().get())
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
        self.scope.hold_watching(owner.observe::<R>(move |update| {
            if awake.now() {
                callback(update);
            }
        }));
    }

    /// Opens the feed of `T` here: the writer is this feature's alone, and
    /// what it [`feed`](FeedWriter::feed)s is for others to follow.
    ///
    /// When following it starts and stops matters to whoever produces the
    /// value, answer [`Followed<T>`](guinea_core::feed::Followed) here.
    ///
    /// # Panics
    ///
    /// When this scope already has a feed of `T`, naming where it was opened.
    #[track_caller]
    pub fn feed<T: Send + Sync + 'static>(&self) -> FeedWriter<T> {
        self.section.feed::<T>()
    }

    /// Hands `run` every value `feed` takes while this scope is awake,
    /// starting with the one it has, for as long as this scope lives.
    ///
    /// Asleep, it is told nothing; woken, it is handed the value once if the
    /// feed moved meanwhile.
    pub fn follow<T: Send + Sync + 'static>(&self, feed: Feed<T>, run: impl Fn(Arc<T>) + 'static) {
        feed.follow(self.scope, run);
    }

    /// Hears `M` on the bus of this scope's tree, for as long as this scope
    /// lives and while it is awake.
    #[track_caller]
    pub fn subscribe<M: Event>(&self, callback: impl Fn(M) + 'static) {
        self.section.note_listener(name::<M>(), None);
        self.scope
            .hold_hearing(self.scope.bus().subscribe_fn(self.while_awake(callback)));
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
    pub fn spawn<A: ManagedActor + 'static>(&self, actor: A) -> Addr<A> {
        Addr::new(actor, &Home::in_section(self.section))
    }

    /// Hands `to` every item `source` yields, for as long as this scope lives
    /// and while it is awake: `to` is an actor that handles it, or a function
    /// that takes it. See [`guinea_core::sources`].
    #[track_caller]
    pub fn pour<S>(&self, source: S, to: impl Receives<S::Item>) -> Source
    where
        S: Stream + Send + 'static,
        S::Item: Send + 'static,
    {
        self.section.pour(source, to)
    }

    /// Hands `to` a `message()` every `period`, for as long as this scope
    /// lives and while it is awake: `to` is an actor that handles it, or a
    /// function that takes it.
    #[track_caller]
    pub fn every<M: 'static>(
        &self,
        period: impl Into<Period>,
        message: impl Fn() -> M + 'static,
        to: impl Receives<M>,
    ) -> Source {
        self.section.every(period, message, to)
    }

    /// Runs `run` every `period`, for as long as this scope lives and while
    /// it is awake.
    #[track_caller]
    pub fn repeat(&self, period: impl Into<Period>, run: impl FnMut() + 'static) -> Source {
        self.section.repeat(period, run)
    }
}
