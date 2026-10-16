//! A value one feature offers others: one writer, the latest value, read up
//! the scope tree.
//!
//! State that crosses feature boundaries does not go over the bus, which has
//! no current value, loses what was said while a page slept, and lets anyone
//! publish. A feature opens a feed of `T` with `cx.feed::<T>()` and is the
//! only one handed its [`FeedWriter`]; whatever produces the value - an
//! actor, a task, a thread - gets a clone of it. Everyone else gets the
//! [`Feed`], which is `Copy`, and follows it with a closure.
//!
//! A follower is handed each value once, after the turn that set it, with
//! the state that turn changed. While its scope sleeps it is told nothing; on
//! waking it is handed the value once if the feed moved meanwhile. Writes
//! from another thread are one hop to the UI thread however many there were
//! before it ran, and the followers get whichever came last.
//!
//! Whether anyone follows is the producer's business: when the first awake
//! follower comes and the last one goes, [`Followed<T>`] is sent into the
//! section that opened the feed - if anything there answers it.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::ThreadId;

use arc_swap::ArcSwapOption;

use crate::scope::{Awake, Scope, Section, next_key};
use crate::short_type_name;
use crate::ui_thread::try_invoke_on_ui;

/// Where a feed is written: handed only to the feature that opened it, and
/// handed on by clone to whatever produces the value, on any thread.
pub struct FeedWriter<T> {
    shared: Arc<Shared<T>>,
}

/// A feed to read and follow. Names its scope and does not keep it.
pub struct Feed<T> {
    home: Scope,
    /// What its scope holds it under.
    key: usize,
    of: PhantomData<fn() -> T>,
}

/// Sent into the section that opened a feed of `T` when it gains its first
/// awake follower, `Followed(true, ..)`, and when it loses its last one.
pub struct Followed<T>(pub bool, PhantomData<fn() -> T>);

/// What the writer and the UI thread share.
struct Shared<T> {
    current: ArcSwapOption<Stamped<T>>,
    versions: AtomicU64,
    /// A hop to the UI thread is on its way, and will hand on whatever is
    /// current when it lands.
    hopping: AtomicBool,
    /// Its scope was removed.
    gone: AtomicBool,
    home: Scope,
    key: usize,
    /// The thread the feed was opened on, the UI thread.
    ui: ThreadId,
}

struct Stamped<T> {
    version: u64,
    value: Arc<T>,
}

/// A feed as its scope holds it.
struct Fed<T> {
    shared: Arc<Shared<T>>,
    /// Where [`Followed`] is sent.
    section: Section,
    opened: &'static Location<'static>,
    followers: RefCell<Vec<Rc<Follower<T>>>>,
    /// What was last said in [`Followed`].
    followed: Cell<bool>,
}

struct Follower<T> {
    key: usize,
    /// The version it was last handed; 0 for none.
    seen: Cell<u64>,
    awake: Awake,
    /// Its scope went; a delivery already under way skips it.
    gone: Cell<bool>,
    run: Box<dyn Fn(Arc<T>)>,
}

impl<T> Clone for FeedWriter<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Clone for Feed<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Feed<T> {}

impl<T> std::fmt::Debug for Feed<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Feed<{}>({:?})", short_type_name::<T>(), self.home)
    }
}

impl<T> std::fmt::Debug for FeedWriter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FeedWriter<{}>({:?})", short_type_name::<T>(), self.shared.home)
    }
}

impl<T> std::fmt::Debug for Followed<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Followed<{}>({})", short_type_name::<T>(), self.0)
    }
}

/// A feed as the scope that opened it holds it, whatever it carries.
pub(crate) trait Opened {
    fn of(&self) -> TypeId;

    /// Where it was opened.
    fn opened(&self) -> &'static Location<'static>;

    /// Its scope let go of it: a value set from another thread is dropped
    /// from now on.
    fn close(&self);

    fn into_any(self: Rc<Self>) -> Rc<dyn Any>;
}

impl<T: Send + Sync + 'static> Opened for Fed<T> {
    fn of(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn opened(&self) -> &'static Location<'static> {
        self.opened
    }

    fn close(&self) {
        self.shared.gone.store(true, Ordering::Release);
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

impl Section {
    /// Opens the feed of `T` in this section's scope; [`Followed<T>`] is sent
    /// here.
    ///
    /// # Panics
    ///
    /// When the scope already has one, naming where it was opened, and when
    /// the scope is gone.
    #[track_caller]
    pub fn feed<T: Send + Sync + 'static>(&self) -> FeedWriter<T> {
        let opened = Location::caller();
        let home = self.scope();
        let key = next_key();

        let shared = Arc::new(Shared {
            current: ArcSwapOption::empty(),
            versions: AtomicU64::new(0),
            hopping: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            home,
            key,
            ui: std::thread::current().id(),
        });
        let fed = Rc::new(Fed {
            shared: shared.clone(),
            section: *self,
            opened,
            followers: RefCell::default(),
            followed: Cell::new(false),
        });

        if let Err(first) = home.hold_feed(key, fed) {
            panic!(
                "a feed of `{}` is opened at {opened}, and {home:?} already opened one at \
                 {first} - a scope has one feed of each type",
                short_type_name::<T>()
            );
        }

        FeedWriter { shared }
    }
}

impl<T: Send + Sync + 'static> FeedWriter<T> {
    /// Makes `value` the feed's value. Setting the value it has already is
    /// nothing.
    ///
    /// On the UI thread the followers are handed it when the turn ends; from
    /// another thread, in one hop to the UI thread for however many values
    /// were set before it ran.
    ///
    /// # Panics
    ///
    /// On the UI thread, when the feed's scope is gone. From another thread
    /// the value is dropped with a warning instead: a producer cannot know
    /// when the page it was producing for left.
    pub fn set(&self, value: impl Into<Arc<T>>) {
        let shared = &*self.shared;
        let on_ui = std::thread::current().id() == shared.ui;

        if on_ui && !shared.home.is_alive() {
            panic!(
                "setting the feed of `{}` in {:?}, which was removed",
                short_type_name::<T>(),
                shared.home
            );
        }

        if !on_ui && shared.gone.load(Ordering::Acquire) {
            tracing::warn!(
                feed = short_type_name::<T>(),
                "a feed whose scope was removed was set from another thread; dropped"
            );
            return;
        }

        let value = value.into();
        let same = shared
            .current
            .load()
            .as_ref()
            .is_some_and(|now| Arc::ptr_eq(&now.value, &value));
        if same {
            return;
        }

        let version = shared.versions.fetch_add(1, Ordering::Relaxed) + 1;
        shared.current.store(Some(Arc::new(Stamped { version, value })));

        if on_ui {
            crate::notify::feed(shared.home, shared.key, deliver::<T>);
            return;
        }

        if shared.hopping.swap(true, Ordering::AcqRel) {
            return;
        }

        let landed = self.shared.clone();
        let hop = move || {
            landed.hopping.swap(false, Ordering::AcqRel);
            if landed.home.is_alive() {
                crate::notify::feed(landed.home, landed.key, deliver::<T>);
            }
        };
        if try_invoke_on_ui(hop).is_err() {
            shared.hopping.store(false, Ordering::Release);
            tracing::warn!(
                feed = short_type_name::<T>(),
                "a feed was set from another thread with no UI thread to hand it to"
            );
        }
    }

    /// The feed, to read and follow.
    pub fn feed(&self) -> Feed<T> {
        Feed {
            home: self.shared.home,
            key: self.shared.key,
            of: PhantomData,
        }
    }
}

pub(crate) mod above {
    use std::any::TypeId;

    use crate::scope::Scope;

    /// Whether a scope above `scope` opened a feed of the type `of`.
    pub fn opened_above(scope: Scope, of: TypeId) -> bool {
        scope
            .ancestors()
            .iter()
            .any(|above| above.feed_key(of).is_some())
    }
}

fn deliver<T: Send + Sync + 'static>(home: Scope, key: usize) {
    Feed::<T> {
        home,
        key,
        of: PhantomData,
    }
    .deliver();
}

impl<T: Send + Sync + 'static> Feed<T> {
    /// The feed of `T` the nearest scope above `scope` opened.
    pub fn above(scope: Scope) -> Option<Self> {
        let of = TypeId::of::<T>();
        let (home, key) = scope
            .ancestors()
            .into_iter()
            .find_map(|above| Some((above, above.feed_key(of)?)))?;

        Some(Self {
            home,
            key,
            of: PhantomData,
        })
    }

    /// Its value now, `None` before the first `set` and once its scope is
    /// gone.
    pub fn get(&self) -> Option<Arc<T>> {
        let fed = self.fed()?;
        let now = fed.shared.current.load();
        now.as_ref().map(|now| now.value.clone())
    }

    /// Hands `run` every value the feed takes while `by` is awake, starting
    /// with the one it has, for as long as `by` lives.
    ///
    /// # Panics
    ///
    /// When the feed's scope or `by` is gone.
    pub fn follow(&self, by: Scope, run: impl Fn(Arc<T>) + 'static) {
        let fed = self
            .fed()
            .unwrap_or_else(|| panic!("following {self:?}, whose scope was removed"));

        let key = next_key();
        let feed = *self;
        by.on_wake(move || feed.woke(key));
        by.on_sleep(move || feed.recount());
        by.on_teardown(move || feed.unfollow(key));

        let follower = Rc::new(Follower {
            key,
            seen: Cell::new(0),
            awake: by.awake(),
            gone: Cell::new(false),
            run: Box::new(run),
        });
        fed.followers.borrow_mut().push(follower.clone());
        drop(fed);

        if follower.awake.now() {
            self.hand(&follower);
        }
        self.recount();
    }

    fn fed(&self) -> Option<Rc<Fed<T>>> {
        self.home.held_feed(self.key)?.downcast::<Fed<T>>().ok()
    }

    /// Hands `follower` the value now, unless it has seen it.
    fn hand(&self, follower: &Follower<T>) {
        let Some(fed) = self.fed() else { return };
        let Some(now) = fed.shared.current.load_full() else {
            return;
        };
        drop(fed);

        if follower.gone.get() || follower.seen.get() == now.version {
            return;
        }

        follower.seen.set(now.version);
        (follower.run)(now.value.clone());
    }

    /// Hands every awake follower the value now.
    fn deliver(&self) {
        let Some(fed) = self.fed() else { return };
        let followers = fed.followers.borrow().clone();
        drop(fed);

        for follower in followers.iter().filter(|follower| follower.awake.now()) {
            self.hand(follower);
        }
    }

    fn woke(&self, key: usize) {
        let Some(fed) = self.fed() else { return };
        let follower = fed
            .followers
            .borrow()
            .iter()
            .find(|follower| follower.key == key)
            .cloned();
        drop(fed);

        if let Some(follower) = follower {
            self.hand(&follower);
        }
        self.recount();
    }

    fn unfollow(&self, key: usize) {
        let Some(fed) = self.fed() else { return };
        fed.followers.borrow_mut().retain(|follower| {
            let leaving = follower.key == key;
            if leaving {
                follower.gone.set(true);
            }
            !leaving
        });
        drop(fed);

        self.recount();
    }

    /// Says [`Followed`] when whether anyone awake follows changed.
    fn recount(&self) {
        let Some(fed) = self.fed() else { return };
        let followed = fed
            .followers
            .borrow()
            .iter()
            .any(|follower| follower.awake.now() && !follower.gone.get());
        if followed == fed.followed.replace(followed) {
            return;
        }

        let answer = fed.section.answerer::<Followed<T>>();
        drop(fed);
        if let Some(answer) = answer {
            answer(Followed(followed, PhantomData));
        }
    }
}
