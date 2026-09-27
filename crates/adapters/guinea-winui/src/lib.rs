//! guinea on windows-reactor: what a view is, how a segment is mounted, and
//! what a view reads state through.
//!
//! It has a run loop again. Under the render-and-hook API there was nowhere to
//! put the application, so installing it happened inside the first render and
//! this backend never labelled its root; a window is a component root now, and
//! [`run`] is an ordinary `run` like the other four backends have.

mod devtools;
mod dispatching;
#[cfg(feature = "harness")]
pub mod harness;
mod mark;
mod run;
mod winui;

pub use guinea_app::feature::FeatureInitContext;
pub use guinea_core::guard::{Ask, Verdict};
pub use guinea_macros::{winui_layout as layout, winui_page as page};
pub use mark::MarkExt;
pub use run::{MAIN, Window, run, window};
pub use winui::*;
