//! The build half of the Slint backend: the route tree, written out as
//! `.slint`.
//!
//! Slint cannot embed a Rust-compiled component into another one, so the tree
//! a route shows has to exist up front - every page in it, with the route
//! choosing which branch is alive. Written by hand that is the same tree twice,
//! once in `routes!` and once in markup, kept in step by hand.
//!
//! So it is generated. [`compile`] reads the `routes!` declaration, finds the
//! `.slint` each page and layout is written in, and emits:
//!
//! - `route-tree.slint`: one component holding the whole tree, with an
//!   `in property <RouteId> route` selecting the branch, and the `RouteId`
//!   enum naming the branches;
//! - `route_id.rs`: the matching `route_id(&Route) -> RouteId`.
//!
//! ```no_run
//! // build.rs
//! fn main() -> anyhow::Result<()> {
//!     guinea_slint_build::compile("src")
//! }
//! ```
//!
//! The application keeps its own window, which imports the generated tree:
//!
//! ```ignore
//! // src/app.slint
//! import { RouteTree } from "route-tree.slint";
//!
//! import { RouteTree, RouteId } from "route-tree.slint";
//!
//! export component AppWindow inherits Window {
//!     in property <RouteId> route;
//!     title: "my application";
//!     RouteTree { width: root.width; height: root.height; route: root.route; }
//! }
//! ```
//!
//! # What it expects
//!
//! - `<root>/routes.rs` declares the tree, and `<root>/app.slint` is the
//!   window;
//! - every page and layout in the declaration has a component of the same name
//!   exported from some `.slint` under `<root>` - `page(Processes, ...)` is
//!   drawn by `export component Processes`, wherever that file sits.

mod scan;
mod tree;

use std::fs;
use std::path::Path;

use anyhow::Context;
use guinea_codegen::Build;

/// The window, by convention, next to the routes it shows.
const APP: &str = "app.slint";
/// Where the route tree is declared.
const ROUTES: &str = "routes.rs";
/// What the generated tree is called, in both languages.
const GENERATED_SLINT: &str = "route-tree.slint";
const GENERATED_RUST: &str = "route_id.rs";
/// The enum, in a file of its own so that a layout can import it without
/// importing the tree that holds the layout.
pub(crate) const IDS: &str = "route-id.slint";

/// Generates the route tree from `<root>/routes.rs` and compiles
/// `<root>/app.slint` against it.
pub fn compile(root: impl AsRef<Path>) -> anyhow::Result<()> {
    let mut build = Build::from_env();

    // The directory rather than the files in it: adding a page is adding files,
    // and cargo has to notice that too.
    let root = build.track(root);

    let declaration = fs::read_to_string(root.join(ROUTES))
        .with_context(|| format!("{}: no route declaration", root.join(ROUTES).display()))?;
    let routes = guinea_route_dsl::find_in_source(&declaration).with_context(|| {
        format!(
            "{}: no `routes! {{ ... }}` in it",
            root.join(ROUTES).display()
        )
    })?;

    let components = scan::components_under(&root)?;
    let generated = tree::emit(&routes, &components)?;

    build.write(IDS, generated.ids)?;
    build.write(GENERATED_SLINT, generated.slint)?;
    build.write(GENERATED_RUST, generated.rust)?;

    // The generated tree is reached by name, not by path: it lives in OUT_DIR,
    // which the application cannot spell.
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![build.out_dir().to_path_buf()]);
    let app = root.join(APP);
    slint_build::compile_with_config(&app, config)
        .map_err(|e| anyhow::anyhow!("{}: {e}", app.display()))?;

    Ok(())
}
