//! The part of an application that has no toolkit in it: plugins, features,
//! services and teardown.
//!
//! Nothing here knows how anything is drawn. A backend adapter takes an
//! [`app::App`], installs it, and hands the resulting builder a window; see
//! `guinea`'s `run`.

pub mod app;
pub mod feature;
pub mod observability;
