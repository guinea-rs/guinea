use guinea_router::navigator::{Navigator, Routable};
use windows_reactor::Callback;

/// WinUI callbacks for a scope-owned navigator.
pub trait NavigatorExt<R: Routable> {
    /// Navigates to `route` each time the callback is called.
    ///
    /// Uses the navigator's normal history and leave guards. The callback does
    /// not keep the navigator's owning scope alive.
    fn to_callback(&self, route: R) -> Callback<()>;
}

impl<R: Routable> NavigatorExt<R> for Navigator<R> {
    fn to_callback(&self, route: R) -> Callback<()> {
        let navigate = self.to_handler(route);
        Callback::new(move |()| navigate())
    }
}
