use std::ops::{Deref, DerefMut};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use guinea_core::actor::event_bus::{Event, EventBus, GlobalEventBus};
use guinea_core::actor::{UiThreadToken, short_type_name};
use guinea_core::executor::{self, Installed};
use guinea_core::feature::Dispatch;
use guinea_core::scope::{DropGuard, Reducer, Scope, ScopeGuard};
use guinea_core::trace::{self, Cause, Point};

use crate::feature::{Feature, FeatureInitContext, ScopeContext};
use crate::observability::Observed;

use super::acts::{Chain, Recorder};
use super::builder::FeatureBuilder;
use super::host::AppHost;
use super::plugin::{AppFeature, Plugin};
use super::roots::Registration;
use super::runtime;

/// An application without a window, for testing plugins and features.
///
/// Installs run exactly as they would inside [`super::App::run`] - same
/// builder, same registry, same lifecycle - so a plugin can be exercised from
/// its own crate. Nothing here attests to being on a real UI thread: work that
/// actually touches the reactor still needs one.
pub struct TestApp {
    token: UiThreadToken,
    builder: FeatureBuilder,
    _observed: Observed,
}

impl TestApp {
    pub fn new() -> Self {
        let token = UiThreadToken::dangerously_create_token_unchecked();
        let builder = FeatureBuilder::new(token.clone(), AppHost::new());

        Self {
            _observed: builder.observed(),
            builder,
            token,
        }
    }

    pub fn install<P: Plugin>(&mut self, plugin: P) -> anyhow::Result<&mut Self> {
        self.builder.plugin(plugin)?;
        Ok(self)
    }

    pub fn install_feature<F: AppFeature>(&mut self, feature: F) -> anyhow::Result<&mut Self> {
        self.builder.feature(feature)?;
        Ok(self)
    }

    /// Runs cleanups in LIFO order and returns the actors still referenced
    /// afterwards - empty is what a correctly torn-down application looks like.
    pub fn shutdown(self) -> Vec<(&'static str, usize)> {
        runtime::teardown(&self.builder)
    }
}

impl Default for TestApp {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for TestApp {
    type Target = FeatureBuilder;

    fn deref(&self) -> &FeatureBuilder {
        &self.builder
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut FeatureBuilder {
        &mut self.builder
    }
}

/// Features, their actors and everything those spawn, on this one thread and
/// in an order a seed decides.
///
/// A segment without a router: one scope that features install into, the way
/// a page installs them. Background tasks and answers bound for the UI thread
/// do not run until [`settled`](Self::settled), and then run one at a time -
/// whenever several are ready, the seed picks which goes first. The same seed
/// picks the same order every time.
///
/// Time is the executor's: a `tokio::time::sleep`, `interval` or `timeout` in
/// the code under test is made against a paused clock, which moves only by
/// [`advance`](Self::advance).
pub struct Harness {
    segment: FeatureInitContext,
    _scope: ScopeGuard,
    _root: Registration,
    app: TestApp,
    recorder: Recorder,
    executor: Installed,
}

/// How far the clock may run ahead to finish an action before the harness
/// gives up on it.
const HORIZON: Duration = Duration::from_secs(24 * 60 * 60);

impl Harness {
    pub fn new(seed: u64) -> Self {
        let executor = executor::install(seed);
        GlobalEventBus::replace_for_test();
        let app = TestApp::new();
        let root = Registration::open();
        let scope = app.host().scope().child().guard();
        scope.set_window(root.id().get());

        let segment = FeatureInitContext {
            scope_cx: ScopeContext {
                scope: scope.scope(),
                token: app.token.clone(),
                services: app.services.clone(),
            },
            cursor: 0,
            root: root.id(),
            event_bus: Rc::new(EventBus::for_root(root.id().get())),
        };

        Self {
            segment,
            _scope: scope,
            _root: root,
            app,
            recorder: Recorder::start(),
            executor,
        }
    }

    pub fn seed(&self) -> u64 {
        self.executor.seed()
    }

    /// The application around the harness, as a window is opened from it -
    /// see [`FeatureHost::under`](crate::feature::FeatureHost::under).
    pub fn application(&self) -> ScopeContext {
        ScopeContext::clone(&self.app)
    }

    /// Installs a plugin into the application around the harness. What it
    /// provides reaches every segment, so install it before whatever reads it.
    pub fn plugin<P: Plugin>(&mut self, plugin: P) -> anyhow::Result<&mut Self> {
        self.app.install(plugin)?;
        Ok(self)
    }

    /// Provides one service, as a plugin would - for a test that needs a
    /// value and no plugin around it.
    pub fn provide<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
        self.app.provide(value);
        self
    }

    /// Installs the application's own wiring, as [`plugin`](Self::plugin).
    pub fn feature<F: AppFeature>(&mut self, feature: F) -> anyhow::Result<&mut Self> {
        self.app.install_feature(feature)?;
        Ok(self)
    }

    /// Publishes `event` on the global bus the way it lands on the UI thread,
    /// and hands back what it set off.
    pub fn publish<M: Event>(&self, event: M) -> Act<'_> {
        self.record(short_type_name::<M>(), || GlobalEventBus::bus().publish(event))
    }

    /// Runs `done` as one action called `name`, and hands back what it set
    /// off - for a backend's harness to make a click or a key an action like
    /// any other.
    pub fn record(&self, name: &'static str, done: impl FnOnce()) -> Act<'_> {
        let from = self.recorder.heard();

        {
            let _action = trace::enter(|| Point::Action { message: name });
            done();
        }

        let cause = self
            .recorder
            .action_since(from)
            .expect("what was done is recorded as an action");

        Act {
            harness: self,
            cause,
        }
    }

    /// The harness's own segment - the one [`install`](Self::install) and
    /// the rest act on.
    pub fn segment(&self) -> Segment<'_> {
        Segment {
            harness: self,
            cx: self.segment.clone(),
            _owned: None,
        }
    }

    /// A segment over `scope`, which something else installed and owns: a
    /// routed page, layout or part, for a test to read and drive what it
    /// claimed. Dropping it leaves the scope where it is.
    pub fn over(&self, scope: Scope) -> Segment<'_> {
        Segment {
            harness: self,
            cx: FeatureInitContext {
                scope_cx: ScopeContext {
                    scope,
                    ..self.segment.scope_cx.clone()
                },
                ..self.segment.clone()
            },
            _owned: None,
        }
    }

    fn root(&self) -> Segment<'_> {
        self.segment()
    }

    /// Installs `F` into the harness's own segment, which keeps it for as
    /// long as the harness lives.
    pub fn install<F: Feature>(&self, params: &F::Params) -> anyhow::Result<()> {
        self.root().install::<F>(params)
    }

    /// What a page reading `R` is handed to act with: its actions go to the
    /// feature that owns `R`.
    pub fn dispatch<R: Reducer>(&self) -> Dispatch {
        self.root().dispatch::<R>()
    }

    /// Acts on the feature that owns `R`, as [`dispatch`](Self::dispatch)
    /// does, and hands back what the action set off - to wait for, or to
    /// read.
    pub fn act<R: Reducer>(&self, action: impl Sized + 'static) -> Act<'_> {
        self.root().act::<R>(action)
    }

    /// `R` as it is now.
    pub fn state<R: Reducer>(&self) -> Rc<R> {
        self.root().state::<R>()
    }

    /// A segment below the harness's own, the way a page sits below a
    /// layout: it reads what the segment above exports, and leaving it tears
    /// down only what it installed.
    pub fn child(&self) -> Segment<'_> {
        self.root().child()
    }

    /// Runs everything that was set off until nothing is left to run, and
    /// says how many pieces ran. The clock does not move: a task waiting on a
    /// timer waits.
    pub fn settled(&self) -> usize {
        self.executor.run_until_parked()
    }

    /// Moves the clock on by `by`, one timer at a time, with everything each
    /// timer sets off run before the next one fires - and no real waiting.
    pub fn advance(&self, by: Duration) {
        self.executor.advance(by);
    }

    /// Tasks still waiting: on a timer the clock has not reached, or on
    /// something nothing here will wake.
    pub fn stuck(&self) -> usize {
        self.executor.stuck()
    }
}

impl Drop for Harness {
    /// Tears the application down the way leaving `run` does: the segment's
    /// features first, then every plugin's cleanups. A plugin that puts
    /// something process-wide in place takes it away again, so the next seed
    /// can put it back.
    fn drop(&mut self) {
        self.segment.scope.remove();
        runtime::teardown(&self.app.builder);
    }
}

/// One segment of a [`Harness`]: a scope features install into, under the
/// segment it was made from.
///
/// A [`child`](Self::child) owns its scope. Leaving it - or dropping it -
/// tears the scope down the way a navigation does: its actors are disposed and
/// their tasks cancelled, while the segments above carry on. The harness's own
/// segment goes with the harness.
pub struct Segment<'h> {
    harness: &'h Harness,
    cx: FeatureInitContext,
    _owned: Option<ScopeGuard>,
}

impl<'h> Segment<'h> {
    /// Installs `F` here, for as long as the segment lives.
    pub fn install<F: Feature>(&self, params: &F::Params) -> anyhow::Result<()> {
        let installed = self.cx.install::<F>(params)?;
        self.cx.scope.own(DropGuard(installed));
        Ok(())
    }

    /// Where a page here reads `R` from: this scope if it claimed `R`, or the
    /// nearest one above that exports it - the rule the router reads by.
    fn owner<R: Reducer>(&self) -> Scope {
        self.cx.scope.owner_of::<R>().unwrap_or_else(|| {
            panic!(
                "nothing here claims {} and nothing above exports it - install the feature \
                 that does",
                std::any::type_name::<R>()
            )
        })
    }

    /// What a page here reading `R` is handed to act with.
    pub fn dispatch<R: Reducer>(&self) -> Dispatch {
        Dispatch::owning::<R>(self.owner::<R>())
    }

    /// See [`Harness::act`].
    pub fn act<R: Reducer>(&self, action: impl Sized + 'static) -> Act<'h> {
        let named = std::any::type_name_of_val(&action);
        let recorder = &self.harness.recorder;
        let from = recorder.heard();

        self.dispatch::<R>().emit(action);

        let cause = recorder.action_since(from).unwrap_or_else(|| {
            panic!(
                "nothing in the feature that owns {} answers {named}",
                std::any::type_name::<R>()
            )
        });

        Act {
            harness: self.harness,
            cause,
        }
    }

    /// `R` as a page here reads it: from this segment, or the nearest above
    /// that holds it.
    pub fn state<R: Reducer>(&self) -> Rc<R> {
        self.owner::<R>().state::<R>().borrow().clone()
    }

    /// A segment below this one.
    pub fn child(&self) -> Segment<'h> {
        let scope = self.cx.scope.child().guard();

        Segment {
            harness: self.harness,
            cx: FeatureInitContext {
                scope_cx: ScopeContext {
                    scope: scope.scope(),
                    ..self.cx.scope_cx.clone()
                },
                cursor: self.cx.cursor + 1,
                ..self.cx.clone()
            },
            _owned: Some(scope),
        }
    }

    /// A service the harness was provided, as a feature installing here
    /// would [`require`](FeatureInitContext::require) it.
    pub fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        self.cx.require::<T>()
    }

    /// See [`FeatureInitContext::try_require`].
    pub fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.cx.try_require::<T>()
    }

    /// Tears the segment down, as leaving a page does.
    pub fn leave(self) {}

    /// What a feature installing here is handed - for a backend's own
    /// harness to mount a page into this segment.
    pub fn context(&self) -> &FeatureInitContext {
        &self.cx
    }

    /// The harness this segment belongs to.
    pub fn harness(&self) -> &'h Harness {
        self.harness
    }
}

impl crate::services::Services for Segment<'_> {
    fn try_require<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        Segment::try_require(self)
    }

    fn require<T: Send + Sync + 'static>(&self) -> anyhow::Result<Arc<T>> {
        Segment::require(self)
    }
}

/// One action, and everything it set off.
pub struct Act<'h> {
    harness: &'h Harness,
    cause: Cause,
}

impl Act<'_> {
    pub fn cause(&self) -> Cause {
        self.cause
    }

    /// Whether everything the action set off has finished.
    pub fn is_settled(&self) -> bool {
        self.harness.recorder.is_settled(self.cause)
    }

    /// Runs until everything the action set off has finished, and no longer.
    ///
    /// Work it did not set off runs too, in the seed's order, because it is
    /// ready alongside - but a timer that nothing here waits on does not move
    /// the clock. When what the action set off waits on time, the clock goes
    /// to each next timer until it is done, and whatever else those timers
    /// set off runs as well.
    ///
    /// What is ready runs before the action is judged finished. A hop to the
    /// UI thread - `GlobalEventBus::publish` from an actor takes one - is not
    /// traced as work of its own, so until it has run, nothing says the
    /// action is waiting on it.
    pub fn settle(&self) -> &Self {
        loop {
            if self.harness.executor.step() {
                continue;
            }

            if self.is_settled() {
                return self;
            }

            if !self.harness.executor.wake_next(HORIZON) {
                panic!(
                    "the action never finished: something it set off waits on what nothing \
                     here will wake, and no timer is due within {HORIZON:?}\n{:#?}",
                    self.chain()
                );
            }
        }
    }

    /// What it set off so far, as a tree.
    pub fn chain(&self) -> Chain {
        self.harness.recorder.chain(self.cause)
    }
}

/// Runs `test` as [`check`] does, one at a time among the tests in this
/// process that name the same `key`.
///
/// For what one process has one of - a global store, a named pipe, a port -
/// which tests running side by side on their own threads would otherwise
/// fight over. Held until the last seed's harness is gone, its teardown
/// included.
pub fn check_exclusive(key: &'static str, iterations: u64, test: impl Fn(&mut Harness)) {
    static KEYS: Mutex<Vec<(&str, &Mutex<()>)>> = Mutex::new(Vec::new());

    let held = {
        let mut keys = KEYS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match keys.iter().find(|(known, _)| *known == key) {
            Some((_, held)) => *held,
            None => {
                let held: &'static Mutex<()> = Box::leak(Box::new(Mutex::new(())));
                keys.push((key, held));
                held
            }
        }
    };

    let _turn = held.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    check(iterations, test);
}

/// Runs `test` once per seed, `0..iterations`, each on a fresh [`Harness`];
/// with `SEED` set, only that seed.
///
/// A seed that fails is named in the panic, and `SEED=<n>` replays exactly the
/// order that failed.
pub fn check(iterations: u64, test: impl Fn(&mut Harness)) {
    let seeds: Vec<u64> = match std::env::var("SEED") {
        Ok(seed) => vec![seed.trim().parse().expect("SEED is a number")],
        Err(_) => (0..iterations).collect(),
    };

    for seed in seeds {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut harness = Harness::new(seed);
            test(&mut harness);
        }));

        if let Err(panic) = outcome {
            let said = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
                .unwrap_or_default();

            panic!("{said}\n\nfailed with SEED={seed}; `SEED={seed} cargo test` runs this order again");
        }
    }
}
