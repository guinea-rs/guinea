//! Every live router on this thread, readable without knowing its backend or
//! its route type.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use guinea_app::app::roots::{self, RootId};
use guinea_core::actor::registry::ActorSnapshot;
use guinea_core::devtools::{Change, Panel};
use guinea_core::actor::shape::Declared;
use guinea_core::scope::{DescribedState, Installed, Listener};

use crate::router::{Router, Ui};

/// A router as devtools see it.
pub struct RouterView {
    pub root: RootId,
    pub label: Option<String>,
    /// The backend's type name: `Egui`, `WinUi`.
    pub backend: &'static str,
    pub route: Option<String>,
    pub segments: Vec<SegmentView>,
    pub back: usize,
    pub forward: usize,
    pub pending: Option<String>,
    /// Actors spawned or driven by features under this router.
    pub actors: Vec<ActorSnapshot>,
    /// What the backend offered for this root.
    pub panels: Vec<Panel>,
    /// What is subscribed to this window's bus, by event.
    pub bus: Vec<(&'static str, usize)>,
}

pub struct SegmentView {
    pub name: &'static str,
    /// Matches `Owner::scope` on the actors this segment owns.
    pub scope: usize,
    /// Where `routes!` listed this page or layout.
    pub declared: Option<Declared>,
    /// Where the page or layout itself was written.
    pub written: Option<Declared>,
    pub features: Vec<Installed>,
    pub states: Vec<DescribedState>,
    pub listeners: Vec<Listener>,
}

trait Inspected {
    fn view(&self) -> RouterView;

    fn actor(&self, id: usize) -> Option<ActorSnapshot>;

    fn root_id(&self) -> RootId;

    /// The scopes of the active chain, from the outermost layout down.
    fn scopes(&self) -> Vec<guinea_core::scope::Scope>;
}

thread_local! {
    static ROUTERS: RefCell<Vec<Weak<dyn Inspected>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn register<U: Ui>(router: &Rc<Router<U>>) {
    let weak = Rc::downgrade(&(router.clone() as Rc<dyn Inspected>));
    ROUTERS.with(|routers| routers.borrow_mut().push(weak));

    let root = router.root().get();
    guinea_core::devtools::changed(|| Change::RouterOpened { root });
}

/// A listed router is going: what [`register`] said came, this says went.
pub(crate) fn unregister(root: RootId) {
    guinea_core::devtools::changed(|| Change::RouterClosed { root: root.get() });
}

fn alive() -> Vec<Rc<dyn Inspected>> {
    ROUTERS.with(|routers| {
        let mut routers = routers.borrow_mut();
        routers.retain(|router| router.strong_count() > 0);
        routers.iter().filter_map(Weak::upgrade).collect()
    })
}

/// Every router on this thread that is still alive, in the order they first
/// navigated.
pub fn routers() -> Vec<RouterView> {
    alive().iter().map(|router| router.view()).collect()
}

/// The router of window `root`, as `RootId::get` numbers it; the others are
/// not read.
pub fn router(root: u64) -> Option<RouterView> {
    find(root).map(|router| router.view())
}

fn find(root: u64) -> Option<Rc<dyn Inspected>> {
    alive()
        .into_iter()
        .find(|router| router.root_id().get() == root)
}

/// The actor `id`, read now: one of window `root`'s, or of the application's
/// for `None` - where [`Change::ActorAdded`] said it was.
///
/// [`Change::ActorAdded`]: guinea_core::devtools::Change::ActorAdded
pub fn actor(root: Option<u64>, id: usize) -> Option<ActorSnapshot> {
    match root {
        Some(root) => find(root)?.actor(id),
        None => guinea_app::app::actors::app_actor(id),
    }
}

/// Sends the action registered as `action`, decoded from `json`, to the scope
/// that answers it - under the router of window `root`, or the newest one.
///
/// The page's own scope is asked first and the layouts above it after, the
/// order a page reading the state would find it in. Answers the action's id
/// in the trace.
pub fn act(root: Option<u64>, action: &str, json: &str) -> Result<u64, String> {
    let routers = alive();
    let router = match root {
        Some(root) => routers.iter().find(|router| router.root_id().get() == root),
        None => routers.last(),
    }
    .ok_or_else(|| format!("no window {root:?} is open"))?;

    guinea_core::remote::act_in(&router.scopes(), action, json)
}

/// The name a segment goes by: its type, without the path or the generics.
pub fn short(name: &'static str) -> &'static str {
    let generic = name.find('<').unwrap_or(name.len());
    let start = name[..generic].rfind("::").map_or(0, |at| at + 2);
    &name[start..generic]
}

impl<U: Ui> Inspected for Router<U> {
    fn view(&self) -> RouterView {
        let root = self.root();
        let segments = match (self.active_chain(), self.active_scopes()) {
            (Some(chain), Some(scopes)) => chain
                .iter()
                .zip(scopes.iter())
                .map(|(entry, scope)| SegmentView {
                    name: short((entry.type_name)()),
                    scope: scope.key(),
                    declared: entry.declared,
                    written: entry.written,
                    features: scope.features(),
                    states: scope.describe_states(),
                    listeners: scope.listeners(),
                })
                .collect(),
            _ => Vec::new(),
        };

        let mut actors = self.host().debug_registry().snapshots();
        actors.sort_by_key(|actor| actor.id);

        RouterView {
            root,
            label: roots::label(root),
            backend: short(std::any::type_name::<U>()),
            route: self.described(),
            segments,
            back: self.history_len().0,
            forward: self.history_len().1,
            pending: self.pending().map(|ask| ask.text),
            actors,
            panels: guinea_core::devtools::panels(root.get()),
            bus: self.host().event_bus().subscriptions(),
        }
    }

    fn actor(&self, id: usize) -> Option<ActorSnapshot> {
        self.host().debug_registry().snapshot_of(id)
    }

    fn root_id(&self) -> RootId {
        self.root()
    }

    fn scopes(&self) -> Vec<guinea_core::scope::Scope> {
        self.active_scopes()
            .map(|scopes| scopes.to_vec())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn names_lose_their_path_and_keep_their_generics_out() {
        assert_eq!(short("app::pages::Processes"), "Processes");
        assert_eq!(short("guinea_eframe::Egui"), "Egui");
        assert_eq!(short("a::List<b::Recent>"), "List");
        assert_eq!(short("Plain"), "Plain");
    }
}
