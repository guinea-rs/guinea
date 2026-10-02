use crate::actor::addr::Addr;
use crate::actor::shape::Shape;
use crate::actor::short_type_name;
use crate::actor::traits::ManagedActor;
use crate::devtools::{self, Change};
use parking_lot::RwLock;
use std::any::{Any, TypeId};
use std::collections::HashMap;

#[derive(Default)]
pub struct ActorRegistry {
    actors: RwLock<HashMap<TypeId, Box<dyn Any>>>,
}

// HACK: this registry for test only
unsafe impl Send for ActorRegistry {}
unsafe impl Sync for ActorRegistry {}

impl ActorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<A: 'static>(&self, addr: Addr<A>) {
        let mut actors = self.actors.write();
        actors.insert(TypeId::of::<A>(), Box::new(addr));
    }

    pub fn get<A: 'static>(&self) -> Option<Addr<A>> {
        let actors = self.actors.read();
        actors
            .get(&TypeId::of::<A>())?
            .downcast_ref::<Addr<A>>()
            .cloned()
    }
}

pub struct ActorSnapshot {
    pub id: usize,
    pub type_name: &'static str,
    pub state: String,
    pub shape: Shape,
    pub owner: Owner,
}

/// Where an actor lives, for grouping it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    /// The scope that owns it, as `Scope::key` names it.
    pub scope: Option<usize>,
    /// The feature that created it.
    pub feature: Option<&'static str>,
    /// The reducer it drives, when it was created with `driven_by`.
    pub drives: Option<&'static str>,
}

struct DebugEntry {
    type_name: &'static str,
    shape: Shape,
    owner: Owner,
    snapshot: Box<dyn Fn() -> String>,
}

#[derive(Default)]
pub struct DebugRegistry {
    entries: RwLock<HashMap<usize, DebugEntry>>,
    /// The window it lists the actors of, for [`Change`]; `None` for the
    /// application's.
    root: Option<u64>,
}

impl DebugRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry of window `root`, as `RootId::get` numbers it.
    pub fn for_root(root: u64) -> Self {
        Self {
            entries: RwLock::default(),
            root: Some(root),
        }
    }

    pub fn register<A: std::fmt::Debug + ManagedActor>(&self, addr: &Addr<A>) {
        self.register_owned(addr, Owner::default());
    }

    pub fn register_owned<A: std::fmt::Debug + ManagedActor>(&self, addr: &Addr<A>, owner: Owner) {
        let id = addr.id();
        let type_name = short_type_name::<A>();
        let addr = addr.clone();
        self.entries.write().insert(
            id,
            DebugEntry {
                type_name,
                shape: A::SHAPE,
                owner,
                snapshot: Box::new(move || addr.debug_snapshot()),
            },
        );

        devtools::changed(|| Change::ActorAdded {
            root: self.root,
            id,
            type_name,
            owner,
        });
    }

    pub fn unregister(&self, id: usize) {
        let removed = self.entries.write().remove(&id);
        if removed.is_some() {
            devtools::changed(|| Change::ActorRemoved { root: self.root, id });
        }
    }

    pub fn clear(&self) {
        let removed: Vec<usize> = self.entries.write().drain().map(|(id, _)| id).collect();
        for id in removed {
            devtools::changed(|| Change::ActorRemoved { root: self.root, id });
        }
    }

    pub fn snapshots(&self) -> Vec<ActorSnapshot> {
        self.entries
            .read()
            .iter()
            .map(|(&id, entry)| Self::snapshot(id, entry))
            .collect()
    }

    /// The actor listed as `id`, read now; the others are not read.
    pub fn snapshot_of(&self, id: usize) -> Option<ActorSnapshot> {
        self.entries
            .read()
            .get(&id)
            .map(|entry| Self::snapshot(id, entry))
    }

    fn snapshot(id: usize, entry: &DebugEntry) -> ActorSnapshot {
        ActorSnapshot {
            id,
            type_name: entry.type_name,
            shape: entry.shape,
            owner: entry.owner,
            state: (entry.snapshot)(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Cx, Handler, UiThreadToken};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[derive(Debug)]
    struct Counter(u32);

    struct Bump;

    guinea_macros::actor! {
        Counter {
            handlers {
                Bump
            }
        }
    }

    impl Handler<Bump> for Counter {
        fn handle(&mut self, _: Bump, _cx: Cx<Self, Bump>) {
            self.0 += 1;
        }
    }

    fn counter() -> Addr<Counter> {
        Addr::new_managed_scoped(Counter(0), UiThreadToken::dangerously_create_token_unchecked())
    }

    fn watched(run: impl FnOnce()) -> Vec<Change> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        devtools::watch(move |change| sink.borrow_mut().push(change.clone()));

        run();
        devtools::stop_watching();

        seen.take()
    }

    #[test]
    fn a_registry_tells_devtools_what_came_and_went_and_whose_it_is() {
        let registry = DebugRegistry::for_root(7);
        let addr = counter();
        let id = addr.id();

        let seen = watched(|| {
            registry.register(&addr);
            registry.unregister(id);
            registry.unregister(id);
        });

        assert_eq!(
            seen,
            [
                Change::ActorAdded {
                    root: Some(7),
                    id,
                    type_name: short_type_name::<Counter>(),
                    owner: Owner::default(),
                },
                Change::ActorRemoved { root: Some(7), id },
            ]
        );
    }

    #[test]
    fn a_cleared_registry_tells_of_every_actor_it_held() {
        let registry = DebugRegistry::new();
        let (first, second) = (counter(), counter());
        registry.register(&first);
        registry.register(&second);

        let mut seen = watched(|| registry.clear());
        seen.sort_by_key(|change| format!("{change:?}"));

        let mut expected = vec![
            Change::ActorRemoved { root: None, id: first.id() },
            Change::ActorRemoved { root: None, id: second.id() },
        ];
        expected.sort_by_key(|change| format!("{change:?}"));
        assert_eq!(seen, expected);
    }

    #[test]
    fn nothing_is_built_for_a_change_nobody_watches() {
        let built = Cell::new(false);
        devtools::changed(|| {
            built.set(true);
            Change::TimerStarted { id: 0 }
        });

        assert!(!built.get());
    }

    #[test]
    fn one_actor_is_read_by_its_id_and_the_others_are_not() {
        let registry = DebugRegistry::new();
        let (bumped, other) = (counter(), counter());
        registry.register(&bumped);
        registry.register(&other);
        bumped.send(Bump);

        let snapshot = registry.snapshot_of(bumped.id()).map(|actor| (actor.id, actor.state));

        assert_eq!(snapshot, Some((bumped.id(), format!("{:#?}", Counter(1)))));
        assert!(registry.snapshot_of(usize::MAX).is_none());
    }
}
