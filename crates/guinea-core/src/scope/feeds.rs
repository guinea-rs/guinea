use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::Scope;

/// The feeds a scope opened, each under the type it carries. See
/// [`crate::feed`].
#[derive(Default)]
pub(super) struct Feeds(RefCell<HashMap<TypeId, Rc<dyn Any>>>);

impl Scope {
    /// The feed of the type `of` this scope opened, if it opened one.
    pub(crate) fn opened_feed(&self, of: TypeId) -> Option<Rc<dyn Any>> {
        self.data()?.feeds.0.borrow().get(&of).cloned()
    }

    /// Opens `feed` here under `of`, or hands back the one already open
    /// under it.
    ///
    /// # Panics
    ///
    /// When this scope is gone.
    pub(crate) fn open_feed(&self, of: TypeId, feed: Rc<dyn Any>) -> Result<(), Rc<dyn Any>> {
        let data = self.alive("opening a feed");
        let mut feeds = data.feeds.0.borrow_mut();

        if let Some(open) = feeds.get(&of) {
            return Err(open.clone());
        }
        feeds.insert(of, feed);
        Ok(())
    }
}
