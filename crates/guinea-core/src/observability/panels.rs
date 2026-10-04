//! Panels: what a backend or a plugin knows that the rest of guinea does not,
//! offered for a window or for the whole application, and drawn by a tool
//! without being interpreted.

use std::cell::RefCell;
use std::rc::Rc;

/// Something a backend knows about a window that the rest of guinea does not:
/// its component tree, say. A tool draws it without interpreting it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    pub id: &'static str,
    pub title: &'static str,
    pub nodes: Vec<PanelNode>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelNode {
    pub label: String,
    pub kind: String,
    pub properties: Vec<(String, String)>,
    pub children: Vec<PanelNode>,
}

type Provider = Rc<dyn Fn() -> Option<Panel>>;

thread_local! {
    static PANELS: RefCell<Vec<(u64, Option<u64>, Provider)>> = const { RefCell::new(Vec::new()) };
    static NEXT_PANEL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn offer(root: Option<u64>, provider: impl Fn() -> Option<Panel> + 'static) -> PanelGuard {
    let id = NEXT_PANEL.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    PANELS.with(|panels| panels.borrow_mut().push((id, root, Rc::new(provider))));
    PanelGuard { id }
}

fn built(root: Option<u64>) -> Vec<Panel> {
    let providers: Vec<Provider> = PANELS.with(|panels| {
        panels
            .borrow()
            .iter()
            .filter(|(_, owner, _)| *owner == root)
            .map(|(_, _, provider)| provider.clone())
            .collect()
    });
    providers.iter().filter_map(|provider| provider()).collect()
}

/// Offers a panel for the root numbered `root`, for as long as the guard
/// lives. `provider` is asked only while something is reading.
#[must_use = "the panel is withdrawn when the guard is dropped"]
pub fn contribute(root: u64, provider: impl Fn() -> Option<Panel> + 'static) -> PanelGuard {
    offer(Some(root), provider)
}

/// Offers a panel about the whole application rather than one window: what a
/// plugin holds, say.
#[must_use = "the panel is withdrawn when the guard is dropped"]
pub fn contribute_to_app(provider: impl Fn() -> Option<Panel> + 'static) -> PanelGuard {
    offer(None, provider)
}

/// Every panel offered for `root`, built now.
pub fn for_root(root: u64) -> Vec<Panel> {
    built(Some(root))
}

/// Every panel offered for the application, built now.
pub fn for_app() -> Vec<Panel> {
    built(None)
}

pub struct PanelGuard {
    id: u64,
}

impl Drop for PanelGuard {
    fn drop(&mut self) {
        let id = self.id;
        let _ = PANELS.try_with(|panels| panels.borrow_mut().retain(|(own, _, _)| *own != id));
    }
}
