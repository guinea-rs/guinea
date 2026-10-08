//! guinea on windows-reactor: what a view is, how a segment is mounted, and
//! what a view reads state through.
//!
//! It has a run loop again. Under the render-and-hook API there was nowhere to
//! put the application, so installing it happened inside the first render and
//! this backend never labelled its root; a window is a component root now, and
//! [`run`] is an ordinary `run` like the other four backends have.
//!
//! Empty off Windows, so a workspace that holds it still builds there.

#![cfg(windows)]

mod devtools;
mod dispatching;
#[cfg(feature = "harness")]
pub mod harness;
mod mark;
mod run;
pub mod semantics;
mod slots;
mod winui;

pub use guinea_app::feature::FeatureInitContext;
pub use guinea_core::guard::{Ask, Verdict};
pub use guinea_macros::{winui_layout as layout, winui_page as page};
pub use mark::MarkExt;
pub use run::{MAIN, Window, run, window};
pub use winui::*;

/// The windows-reactor this backend is built on - `windows-reactor-pre`, the
/// reactor at the windows-rs revision guinea is written against. An
/// application draws with this one, so its views and guinea's are one type.
pub use windows_reactor as reactor;
