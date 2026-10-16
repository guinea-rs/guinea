//! The one tool watching an application: the tree of its scopes as it is,
//! then everything that moves in it, taken a batch at a time.
//!
//! What is light - which scopes there are, what is installed and mounted in
//! them, who listens, which actors and sources run - comes in the events.
//! What is heavy - a state's or an actor's value - is only said to have
//! moved, and read when the tool wants it.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::actor::event_bus::{EventBus, HeardBy};
use crate::actor::shape::Declared;
use crate::actor::snapshot::Owner;
use crate::scope::{Listener, Scope, ScopeId};
use crate::sources::SourceInfo;
use crate::trace::{self, Trace};

#[cfg(test)]
mod tests;

/// Something that came, went or moved in the application being watched.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A scope opened under `parent`.
    ScopeOpened { scope: ScopeId, parent: Option<ScopeId> },
    /// A scope went, and what it held went before it; the scopes under it
    /// are told of first.
    ScopeClosed { scope: ScopeId },
    /// A scope went to sleep: what it holds hears nothing until it wakes.
    Slept { scope: ScopeId },
    Woke { scope: ScopeId },
    /// A feature is installed in `scope`.
    Installed {
        scope: ScopeId,
        name: &'static str,
        declared: Option<Declared>,
    },
    /// The feature [`Event::Installed`] told of failed to install, and is not.
    Withdrawn { scope: ScopeId, name: &'static str },
    /// Something in `scope` listens to an event.
    Listened { scope: ScopeId, listener: Listener },
    /// A state in `scope` is there, or reads differently now.
    StateMoved { scope: ScopeId, reducer: &'static str },
    /// An actor came; what it declared is read with it.
    ActorAdded {
        id: usize,
        type_name: &'static str,
        owner: Owner,
    },
    ActorRemoved { id: usize },
    /// An actor handled a message, so its state may read differently now.
    ActorHandled { id: usize },
    SourceOpened(SourceInfo),
    /// A source was given a name, what devtools call it instead of where it
    /// was opened.
    SourceNamed { id: u64, name: &'static str },
    SourceClosed { id: u64 },
    /// Something joined the bus of the tree.
    Subscribed {
        seq: u64,
        event: &'static str,
        by: HeardBy,
        answers: bool,
    },
    Unsubscribed { seq: u64 },
    /// A plugin the application was built with.
    Plugin { name: &'static str },
    /// What draws the application - `"winui"`, `"egui"`.
    Backend { name: &'static str },
    /// A root - a window's tree - opened on `scope`.
    RootOpened { root: u64, scope: ScopeId },
    RootClosed { root: u64 },
    /// The shell named a root: `"main"`, `"settings"`.
    Labelled { root: u64, label: String },
    /// A page or a layout was mounted as `scope`.
    Mounted {
        scope: ScopeId,
        name: &'static str,
        written: Option<Declared>,
    },
    /// The navigator `scope` opened is somewhere else now.
    Navigated { scope: ScopeId, navigation: Navigation },
}

/// Where a navigator is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigation {
    /// The type of the routes it moves over: what tells two navigators one
    /// scope opened apart.
    pub routes: &'static str,
    /// The route it is at, as `Debug` writes it.
    pub route: String,
    pub back: usize,
    pub forward: usize,
    /// The question a guard is waiting on.
    pub pending: Option<String>,
    /// Why the last place it moved to did not mount.
    pub failure: Option<String>,
}

/// What moved since the last [`Watching::take`].
#[derive(Debug, Default)]
pub struct Batch {
    pub events: Vec<Event>,
    /// Oldest first, each with the thread it was made on.
    pub trace: Vec<(u32, Trace)>,
    /// How many trace records were let go to keep the newest.
    pub dropped: u64,
}

/// What a layer above this crate knows about a scope, told again to a tool
/// that starts watching.
pub struct Describer(pub fn(Scope, &mut Vec<Event>));

inventory::collect!(Describer);

/// A tool already watches this thread.
#[derive(Debug)]
pub struct AlreadyWatched;

impl std::fmt::Display for AlreadyWatched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("something already watches the application on this thread - one tool at a time")
    }
}

impl std::error::Error for AlreadyWatched {}

/// The watch [`watch`] started; dropping it ends the watch.
pub struct Watching {
    observer: Rc<Observer>,
    _traced: trace::Observing,
}

struct Observer {
    root: Scope,
    bus: Weak<EventBus>,
    /// This thread, as the trace names it.
    thread: u32,
    bell: Arc<Bell>,
    buffer: RefCell<Buffer>,
}

/// What moved since the last take. What moves often - a state, an actor's -
/// is kept once however often it moved, and comes after the rest.
#[derive(Default)]
struct Buffer {
    events: Vec<Event>,
    handled: Vec<usize>,
    handled_once: HashSet<usize>,
    moved: Vec<(ScopeId, &'static str)>,
    moved_once: HashSet<(ScopeId, &'static str)>,
    trace: VecDeque<(u32, Trace)>,
    dropped: u64,
}

/// Wakes the tool once, until it takes.
struct Bell {
    armed: AtomicBool,
    ring: Box<dyn Fn() + Send + Sync>,
}

impl Bell {
    fn ring(&self) {
        if self.armed.swap(false, Ordering::AcqRel) {
            (self.ring)();
        }
    }
}

static WATCHED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static OBSERVER: RefCell<Option<Rc<Observer>>> = const { RefCell::new(None) };
}

/// Watches the tree `scope` is in: the first [`Watching::take`] hands over
/// the tree as it is, each one after it what moved since. One tool watches
/// the process at a time.
///
/// `woken` is called once whenever something moves and nothing has been
/// taken since the last call - from the thread it moved on, in the middle
/// of whatever moved it. It hands off and returns: it does not take, and
/// calls nothing here.
pub fn watch(
    scope: Scope,
    woken: impl Fn() + Send + Sync + 'static,
) -> Result<Watching, AlreadyWatched> {
    let root = scope.root().filter(|root| root.is_alive()).ok_or(AlreadyWatched)?;
    if WATCHED.swap(true, Ordering::AcqRel) {
        return Err(AlreadyWatched);
    }

    let bell = Arc::new(Bell {
        armed: AtomicBool::new(true),
        ring: Box::new(woken),
    });
    let observer = Rc::new(Observer {
        root,
        bus: Rc::downgrade(&root.bus()),
        thread: trace::thread_id(),
        bell: bell.clone(),
        buffer: RefCell::new(Buffer {
            events: replay(root),
            ..Buffer::default()
        }),
    });

    OBSERVER.with(|slot| slot.replace(Some(observer.clone())));
    let traced = Rc::downgrade(&observer);
    let traced = trace::observe(move |record| {
        if let Some(observer) = traced.upgrade() {
            observer.traced(record);
        }
    });
    trace::ring_on_elsewhere(Some(Arc::new(move || bell.ring())));
    observer.bell.ring();

    Ok(Watching {
        observer,
        _traced: traced,
    })
}

impl Watching {
    /// What moved since the last take; the first time, the tree as it is.
    pub fn take(&self) -> Batch {
        self.observer.bell.armed.store(true, Ordering::Release);

        let mut buffer = self.observer.buffer.borrow_mut();
        let mut events = std::mem::take(&mut buffer.events);
        events.extend(buffer.handled.drain(..).map(|id| Event::ActorHandled { id }));
        buffer.handled_once.clear();
        events.extend(
            buffer
                .moved
                .drain(..)
                .map(|(scope, reducer)| Event::StateMoved { scope, reducer }),
        );
        buffer.moved_once.clear();
        let mut trace = Vec::from(std::mem::take(&mut buffer.trace));
        let mut dropped = std::mem::take(&mut buffer.dropped);
        drop(buffer);

        let elsewhere = trace::take_elsewhere();
        trace.extend(elsewhere.records);
        dropped += elsewhere.dropped;

        Batch {
            events,
            trace,
            dropped,
        }
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        trace::ring_on_elsewhere(None);
        let _ = OBSERVER.try_with(|slot| slot.take());
        WATCHED.store(false, Ordering::Release);
    }
}

impl Observer {
    fn note(&self, event: Event) {
        self.buffer.borrow_mut().note(event);
        self.bell.ring();
    }

    fn traced(&self, record: &Trace) {
        {
            let mut buffer = self.buffer.borrow_mut();
            if buffer.trace.len() == trace::ELSEWHERE_LIMIT {
                buffer.trace.pop_front();
                buffer.dropped += 1;
            }
            buffer.trace.push_back((self.thread, record.clone()));
        }
        self.bell.ring();
    }
}

impl Buffer {
    fn note(&mut self, event: Event) {
        match &event {
            Event::ActorRemoved { id } if self.handled_once.remove(id) => {
                self.handled.retain(|handled| handled != id);
            }
            Event::ScopeClosed { scope } if self.moved.iter().any(|(moved, _)| moved == scope) => {
                self.moved.retain(|(moved, _)| moved != scope);
                self.moved_once.retain(|(moved, _)| moved != scope);
            }
            _ => {}
        }
        self.events.push(event);
    }
}

/// The tree under `root` as it is, told as if it had just happened: each
/// scope before the ones under it, then everyone on the bus.
fn replay(root: Scope) -> Vec<Event> {
    let mut events = Vec::new();

    for (scope, parent) in root.family() {
        events.push(Event::ScopeOpened {
            scope: scope.id(),
            parent: parent.map(|parent| parent.id()),
        });
        if !scope.is_awake() {
            events.push(Event::Slept { scope: scope.id() });
        }
        events.extend(scope.features().into_iter().map(|installed| Event::Installed {
            scope: scope.id(),
            name: installed.name,
            declared: installed.declared,
        }));
        events.extend(scope.listeners().into_iter().map(|listener| Event::Listened {
            scope: scope.id(),
            listener,
        }));
        events.extend(scope.held_states().into_iter().map(|reducer| Event::StateMoved {
            scope: scope.id(),
            reducer,
        }));
        events.extend(scope.listed_actors());
        events.extend(scope.sources_here().into_iter().map(Event::SourceOpened));
        for describer in inventory::iter::<Describer>() {
            (describer.0)(scope, &mut events);
        }
    }

    events.extend(root.bus().subscribed());
    events
}

fn watching(scope: Scope) -> Option<Rc<Observer>> {
    OBSERVER
        .try_with(|slot| {
            slot.borrow()
                .as_ref()
                .filter(|observer| observer.root.shares_tree(&scope))
                .cloned()
        })
        .ok()
        .flatten()
}

/// Tells the tool watching `scope`'s tree, if one does; `event` is not built
/// otherwise.
pub fn tell(scope: Scope, event: impl FnOnce() -> Event) {
    if let Some(observer) = watching(scope) {
        observer.note(event());
    }
}

/// Tells the tool watching the tree whose bus is `bus`, if one does.
pub(crate) fn tell_bus(bus: &EventBus, event: impl FnOnce() -> Event) {
    let observer = OBSERVER
        .try_with(|slot| {
            slot.borrow()
                .as_ref()
                .filter(|observer| std::ptr::eq(observer.bus.as_ptr(), bus))
                .cloned()
        })
        .ok()
        .flatten();

    if let Some(observer) = observer {
        observer.note(event());
    }
}

/// The state `reducer` in `scope` is there, or reads differently now.
pub(crate) fn moved(scope: Scope, reducer: &'static str) {
    let Some(observer) = watching(scope) else {
        return;
    };

    let key = (scope.id(), reducer);
    {
        let mut buffer = observer.buffer.borrow_mut();
        if !buffer.moved_once.insert(key) {
            return;
        }
        buffer.moved.push(key);
    }
    observer.bell.ring();
}

/// The actor `id`, held in `scope`, handled a message.
pub(crate) fn handled(scope: Scope, id: usize) {
    let Some(observer) = watching(scope) else {
        return;
    };

    {
        let mut buffer = observer.buffer.borrow_mut();
        if !buffer.handled_once.insert(id) {
            return;
        }
        buffer.handled.push(id);
    }
    observer.bell.ring();
}
