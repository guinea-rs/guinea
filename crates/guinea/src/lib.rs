//! guinea, assembled: the agnostic halves plus whichever backends this build
//! renders with.
//!
//! An application depends on this crate and nothing else: the macros are here,
//! and what they expand to finds its way through here too. What almost every
//! file needs is one import, `use guinea::prelude::*;`.
//!
//! A backend arrives as a feature, and while exactly one is enabled it also
//! arrives as [`Backend`] and [`backend`] - which `routes!` targets by default,
//! so a single-backend application names its toolkit nowhere.
//!
//! Enable two and that shorthand goes away on purpose: there is no sensible
//! answer to "the backend" any more, and every route tree has to say which one
//! it is for:
//!
//! ```ignore
//! routes! {
//!     backend = guinea::ratatui::Tui,
//!     Route { .. }
//! }
//! ```
//!
//! With no backend at all what is left is the router, the application runtime
//! and the macros - which is what a port to another toolkit starts from.
//!
//! `winui` is a default feature and means nothing off Windows: there it
//! enables no backend, so a build for Linux with `ratatui` added has ratatui
//! as its one backend.

pub use guinea_router::{enter, headless, link, manifest, restore, router};

/// Watching a running application from outside it: what devtools, a test or
/// a logger read, and what a plugin offers them.
///
/// - [`trace`](observability::trace) - what happened, and what caused it;
/// - [`changes`](observability::changes) - what came or went;
/// - [`panels`](observability::panels) - what a backend or a plugin knows
///   about itself;
/// - [`snapshot`](observability::snapshot) - what is open now, read when
///   asked;
/// - [`act`](observability::act) - sending an action from outside;
/// - [`layer`](observability::layer) - the application's own `tracing`
///   events, into the trace.
pub mod observability {
    pub use guinea_core::observability::{
        LogLayer, Rendering, changes, is_observed, layer, mark_anywhere, panels, profiling,
    };
    pub use guinea_core::trace;
    pub use guinea_router::observability::act;

    /// What is open now: routers, the application, their actors - read when
    /// asked, not kept.
    pub mod snapshot {
        pub use guinea_app::app::installed_plugins;
        pub use guinea_app::observability::{app_actor, app_actors, app_scope};
        pub use guinea_router::observability::{
            RouterView, SegmentView, actor, router, routers, short,
        };
    }
}

#[cfg(all(feature = "winui", target_os = "windows"))]
pub use guinea_winui as winui;

#[cfg(feature = "ratatui")]
pub use guinea_ratatui as ratatui;

#[cfg(feature = "slint")]
pub use guinea_slint as slint;

#[cfg(feature = "eframe")]
pub use guinea_eframe as eframe;

#[cfg(feature = "iced")]
pub use guinea_iced as iced;

/// The backend this build renders with, as a module and as a type.
///
/// Defined only while exactly one backend feature is on. `routes!` falls back
/// to these when a route tree does not name a backend itself.
#[cfg(all(
    all(feature = "winui", target_os = "windows"),
    not(any(
        feature = "ratatui",
        feature = "slint",
        feature = "eframe",
        feature = "iced"
    ))
))]
pub use guinea_winui as backend;
#[cfg(all(
    all(feature = "winui", target_os = "windows"),
    not(any(
        feature = "ratatui",
        feature = "slint",
        feature = "eframe",
        feature = "iced"
    ))
))]
pub type Backend = guinea_winui::WinUi;

#[cfg(all(
    feature = "ratatui",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "slint",
        feature = "eframe",
        feature = "iced"
    ))
))]
pub use guinea_ratatui as backend;
#[cfg(all(
    feature = "ratatui",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "slint",
        feature = "eframe",
        feature = "iced"
    ))
))]
pub type Backend = guinea_ratatui::Tui;

#[cfg(all(
    feature = "slint",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "ratatui",
        feature = "eframe",
        feature = "iced"
    ))
))]
pub use guinea_slint as backend;
#[cfg(all(
    feature = "slint",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "ratatui",
        feature = "eframe",
        feature = "iced"
    ))
))]
pub type Backend = guinea_slint::Slint;

#[cfg(all(
    feature = "eframe",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "ratatui",
        feature = "slint",
        feature = "iced"
    ))
))]
pub use guinea_eframe as backend;
#[cfg(all(
    feature = "eframe",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "ratatui",
        feature = "slint",
        feature = "iced"
    ))
))]
pub type Backend = guinea_eframe::Egui;

#[cfg(all(
    feature = "iced",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "ratatui",
        feature = "slint",
        feature = "eframe"
    ))
))]
pub use guinea_iced as backend;
#[cfg(all(
    feature = "iced",
    not(any(
        all(feature = "winui", target_os = "windows"),
        feature = "ratatui",
        feature = "slint",
        feature = "eframe"
    ))
))]
pub type Backend = guinea_iced::Iced;

/// Stands in for `Backend` when more than one backend is enabled.
///
/// It deliberately implements nothing: the error an application gets is that
/// its routes are not for a backend, which is exactly the mistake - and the
/// `Ui` trait's own diagnostic says how to name one.
#[cfg(any(
    all(
        all(feature = "winui", target_os = "windows"),
        any(
            feature = "ratatui",
            feature = "slint",
            feature = "eframe",
            feature = "iced"
        )
    ),
    all(
        feature = "ratatui",
        any(feature = "slint", feature = "eframe", feature = "iced")
    ),
    all(feature = "slint", any(feature = "eframe", feature = "iced")),
    all(feature = "eframe", feature = "iced"),
))]
pub enum Backend {}

#[cfg(any(
    all(
        all(feature = "winui", target_os = "windows"),
        any(
            feature = "ratatui",
            feature = "slint",
            feature = "eframe",
            feature = "iced"
        )
    ),
    all(
        feature = "ratatui",
        any(feature = "slint", feature = "eframe", feature = "iced")
    ),
    all(feature = "slint", any(feature = "eframe", feature = "iced")),
    all(feature = "eframe", feature = "iced"),
))]
pub mod backend {
    //! Empty on purpose: with more than one backend enabled there is no "the"
    //! backend, so `routes!` has to be told which one - `backend =
    //! guinea::winui::WinUi`, `guinea::ratatui::Tui`, `guinea::slint::Slint`,
    //! `guinea::eframe::Egui` or `guinea::iced::Iced`.
}


pub use guinea_app::{app, app_meta, feature, services, timers};
pub use guinea_app::services::Services;

pub use guinea_codegen as codegen;
pub use guinea_core as core;
pub use guinea_meta as meta;

pub use guinea_core::actor::event_bus::Event;
pub use guinea_core::actor::event_bus::RpcCall as Request;
pub use guinea_core::mark::Mark;
#[allow(deprecated)]
pub use guinea_core::rpc_bind;
pub use guinea_macros::{
    Event, Mark, Remote, Request, actor, app, feature, handler, installs, reducer, routes,
    segment,
};

/// `#[guinea::test]`: one test, run once per seed on a fresh `app::Harness` -
/// which the `test-utils` feature brings.
pub use guinea_macros::test;

/// The error type `install` returns, so that writing a feature does not need
/// a dependency of its own.
#[doc(no_inline)]
pub use guinea_core::__private::anyhow;

/// What a feature, an actor and an application are written with, whatever
/// they draw with: `use guinea::prelude::*;`.
///
/// A backend's own types - `Page`, `PageCx`, `Layout` - are not here; they
/// come from `guinea::backend`, or from the backend's module by name when
/// there are several.
pub mod prelude {
    pub use guinea_app::app::{
        AppFeature, Application, FeatureBuilder, GuineaApp, Installed, Plugin, PluginBuilder,
    };
    pub use guinea_app::feature::{Feature, FeatureInitContext, ScopeContext};
    pub use guinea_app::timers::{Period, Timer};
    pub use guinea_core::actor::event_bus::{Event, GlobalEventBus, Reply};
    pub use guinea_core::actor::{Addr, AsyncContext, Cx, Handler};
    pub use guinea_core::feature::{Bound, Dispatch, Push};
    pub use guinea_core::trace::Bus;
    pub use guinea_core::__private::anyhow;
    pub use guinea_core::{Load, Reducer};
    pub use guinea_macros::{Event, actor, app, feature, handler, installs, reducer, routes};
}
