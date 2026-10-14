//! Every open window on this thread, as a tool reads it: what is mounted in
//! it, where its navigators are, and what runs under it - readable without
//! knowing its backend or its route types.

use guinea_app::app::roots::{self, RootId};
use guinea_core::actor::event_bus::Listening;
use guinea_core::actor::shape::Declared;
use guinea_core::actor::snapshot::ActorSnapshot;
use guinea_core::observability::panels::{self, Panel};
use guinea_core::scope::{DescribedState, Installed, Listener, Scope};

/// A window as a tool reads it.
pub struct WindowView {
    pub root: RootId,
    pub label: Option<String>,
    /// The navigators opened in this window, the window's own first.
    pub navigators: Vec<NavigatorView>,
    /// What is mounted in it, each right after what mounted it.
    pub mounted: Vec<MountedView>,
    /// Actors spawned or driven by features in this window.
    pub actors: Vec<ActorSnapshot>,
    /// What the backend offered for this window.
    pub panels: Vec<Panel>,
    /// Who hears each event on the bus of this window's tree - the same for
    /// every window of one application.
    pub bus: Vec<Listening>,
}

/// Where one navigator is.
pub struct NavigatorView {
    /// The scope that opened it.
    pub scope: Scope,
    /// The route it is at, as `Debug` writes it.
    pub route: String,
    pub back: usize,
    pub forward: usize,
    /// The question a guard is waiting on.
    pub pending: Option<String>,
    /// Why the last place it moved to did not mount.
    pub failure: Option<String>,
}

/// One page or layout mounted in a window.
pub struct MountedView {
    pub name: &'static str,
    /// How many mounts down from the window: what the window mounted is 0.
    pub depth: usize,
    /// Matches `Owner::scope` on the actors this one owns.
    pub scope: Scope,
    /// `false` while it is kept asleep.
    pub awake: bool,
    /// Where the page or layout itself was written.
    pub written: Option<Declared>,
    pub features: Vec<Installed>,
    pub states: Vec<DescribedState>,
    pub listeners: Vec<Listener>,
}

/// Every window on this thread that is open, in the order they opened.
pub fn windows() -> Vec<WindowView> {
    roots::opened().into_iter().map(|(root, window)| view(root, window)).collect()
}

/// The window `root`, as `RootId::get` numbers it; the others are not read.
pub fn window(root: u64) -> Option<WindowView> {
    find(root).map(|(root, window)| view(root, window))
}

fn view(root: RootId, window: Scope) -> WindowView {
    let tree = tree(window);

    let mut navigators = crate::navigator::views(window);
    for (scope, _) in &tree {
        navigators.extend(crate::navigator::views(*scope));
    }

    let mounted = tree
        .into_iter()
        .map(|(scope, depth)| MountedView {
            name: crate::mount::name(scope).map(short).unwrap_or("?"),
            depth,
            scope,
            awake: scope.is_awake(),
            written: crate::mount::written_at(scope),
            features: scope.features(),
            states: scope.describe_states(),
            listeners: scope.listeners(),
        })
        .collect();

    let mut actors = window.actors();
    actors.sort_by_key(|actor| actor.id);

    WindowView {
        root,
        label: roots::label(window),
        navigators,
        mounted,
        actors,
        panels: panels::for_root(root.get()),
        bus: window.bus().listeners(),
    }
}

fn find(root: u64) -> Option<(RootId, Scope)> {
    roots::opened().into_iter().find(|(id, _)| id.get() == root)
}

/// The actor `id` of window `root`, read now. The application's own are read
/// through its scope, with [`app_actor`](guinea_app::observability::app_actor).
pub fn actor(root: u64, id: usize) -> Option<ActorSnapshot> {
    find(root)?.1.actor(id)
}

/// Sends the action registered as `action`, decoded from `json`, to the scope
/// that answers it - in window `root`, or the newest one.
///
/// What is mounted deepest is asked first and what mounted it after, the
/// order a page reading the state would find it in. Answers the action's id
/// in the trace.
pub fn act(root: Option<u64>, action: &str, json: &str) -> Result<u64, String> {
    let open = roots::opened();
    let (_, window) = match root {
        Some(root) => open.into_iter().find(|(id, _)| id.get() == root),
        None => open.into_iter().last(),
    }
    .ok_or_else(|| format!("no window {root:?} is open"))?;

    let mut scopes = vec![window];
    scopes.extend(tree(window).into_iter().filter(|(scope, _)| scope.is_awake()).map(|(scope, _)| scope));
    guinea_core::remote::act_in(&scopes, action, json)
}

/// What is mounted under `parent`, depth first, each with its depth.
fn tree(parent: Scope) -> Vec<(Scope, usize)> {
    let mut found = Vec::new();
    walk(parent, 0, &mut found);
    found
}

fn walk(parent: Scope, depth: usize, into: &mut Vec<(Scope, usize)>) {
    for (child, _) in crate::mount::mounted(parent) {
        into.push((child, depth));
        walk(child, depth + 1, into);
    }
}

/// The name a page or a layout goes by: its type, without the path or the
/// generics.
pub fn short(name: &'static str) -> &'static str {
    let generic = name.find('<').unwrap_or(name.len());
    let start = name[..generic].rfind("::").map_or(0, |at| at + 2);
    &name[start..generic]
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
