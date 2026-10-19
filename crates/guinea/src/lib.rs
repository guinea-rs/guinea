//! guinea, assembled: the agnostic halves plus whichever backends this build
//! renders with.
//!
//! An application depends on this crate and nothing else: the macros are here,
//! and what they expand to finds its way through here too. What almost every
//! file needs is one import, `use guinea::prelude::*;`.
//!
//! A backend arrives as a feature, under its own name: `guinea::winui`,
//! `guinea::ratatui`, `guinea::slint`, `guinea::eframe`, `guinea::iced`.
//! Several may be enabled at once; nothing here picks one.
//!
//! With no backend at all what is left is navigation and mounting, the
//! application runtime and the macros - which is what a port to another
//! toolkit starts from.
//!
//! `winui` is a default feature and means nothing off Windows.

pub use guinea_router::{link, mount, navigator, slot};
#[doc(hidden)]
pub use guinea_router::sample;

/// Watching a running application from outside it: what devtools read, and
/// what a plugin offers them.
///
/// - [`Observed`](observability::Observed) - the one door: the application's
///   tree and everything that moves in it, a [`Batch`](observability::Batch)
///   at a time, and its values read when asked;
/// - [`trace`](observability::trace) - what happened, and what caused it;
/// - [`panels`](observability::panels) - what a backend or a plugin knows
///   about itself;
/// - [`layer`](observability::layer) - the application's own `tracing`
///   events, into the trace.
pub mod observability {
    pub use guinea_app::app::Observed;
    pub use guinea_core::observability::{
        AlreadyWatched, Batch, Event, LogLayer, Navigation, Rendering, Watching, is_observed,
        layer, mark_anywhere, panels,
    };
    pub use guinea_core::scope::ScopeId;
    pub use guinea_core::trace;
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

pub use guinea_app::{app, app_meta, feature};
pub use guinea_core::{actor, executor, guard, sources, ui_thread};

pub use guinea_meta as meta;

/// What macro output names. Not part of the API.
#[doc(hidden)]
pub mod __private {
    pub use guinea_core as core;
}

pub use guinea_core::actor::event_bus::Event;
pub use guinea_core::actor::event_bus::RpcCall as Request;
pub use guinea_core::mark::Mark;
pub use guinea_core::inspect::Inspect;
pub use guinea_macros::{
    Event, Inspect, Mark, Remote, Request, Routable, actor, builds, feature, handler, installs,
    plugin, reducer, slot,
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
/// come from the backend's module: `guinea::winui`, `guinea::eframe`.
pub mod prelude {
    pub use guinea_app::app::{AppFeature, FeatureBuilder, Plugin, PluginBuilder};
    pub use guinea_app::feature::{Feature, FeatureInitContext, Install, ScopeContext};
    pub use guinea_core::sources::{Period, Source, Ticks};
    pub use guinea_core::actor::event_bus::{Event, Reply};
    pub use guinea_core::actor::{Addr, AsyncContext, Cx, Handler, In, Receives};
    pub use guinea_core::feature::{Bound, Dispatch, Push};
    pub use guinea_core::feed::{Feed, FeedWriter, Followed};
    pub use guinea_core::__private::anyhow;
    pub use guinea_core::inspect::Inspect;
    pub use guinea_core::{Load, Reducer};
    pub use guinea_macros::{
        Event, Inspect, actor, builds, feature, handler, installs, plugin, reducer,
    };
}
