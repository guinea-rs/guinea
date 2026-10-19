//! The build half of the Slint backend: the window's `Navigator` global,
//! written from the application's `Route` enum.
//!
//! The markup picks its branch by the route - `if Navigator.route ==
//! RouteId.processes: Processes { }` - and moves the window through
//! callbacks: `Navigator.go-processes()`. Written by hand that is the route
//! twice, once in Rust and once in markup, kept in step by hand. So
//! [`compile`] reads `<root>/routes.rs` and writes, into `OUT_DIR`:
//!
//! - `navigator.slint`: the `RouteId` enum, a `<Place>Params` struct for each
//!   place with parameters, and the `Navigator` global - `route`, each
//!   place's `<place>-params`, `can-go-back`/`can-go-forward`, a guard's
//!   `asking`/`question`/`confirm`/`cancel`, `failure`, and the callbacks
//!   `go-<place>`, `back`, `forward` and `answer`;
//! - `navigator.rs`: `guinea_slint::Shows` for `Route`, which keeps the
//!   global in step with the window's navigator.
//!
//! An application's `build.rs`:
//!
//! ```no_run
//! fn main() -> anyhow::Result<()> {
//!     guinea_slint_build::compile("src")
//! }
//! ```
//!
//! The window, `src/app.slint`, imports the global, and exports it so Rust
//! can reach it:
//!
//! ```slint
//! import { Navigator, RouteId } from "navigator.slint";
//! export { Navigator }
//!
//! export component AppWindow inherits Window {
//!     if Navigator.route == RouteId.processes: Processes { }
//! }
//! ```
//!
//! And the Rust side is included where the route enums and the Slint modules
//! are both in scope:
//!
//! ```ignore
//! slint::include_modules!();
//! use routes::*;
//! include!(concat!(env!("OUT_DIR"), "/navigator.rs"));
//! ```
//!
//! # What it expects
//!
//! - `<root>/routes.rs` declares `#[derive(Routable)] enum Route`, and the route
//!   enums it nests, in the same file;
//! - `<root>/app.slint` is the window;
//! - every parameter is a `String`, a `bool`, an integer or a float - what a
//!   Slint property can carry. Anything else fails the build, naming it.

mod navigator;

use std::fs;
use std::path::Path;

use anyhow::Context;
use guinea_codegen::Build;

/// The window, by convention, next to the routes it shows.
const APP: &str = "app.slint";
/// Where the route is declared.
const ROUTES: &str = "routes.rs";
const GENERATED_SLINT: &str = "navigator.slint";
const GENERATED_RUST: &str = "navigator.rs";

/// Writes the `Navigator` global from `<root>/routes.rs` and compiles
/// `<root>/app.slint` against it.
pub fn compile(root: impl AsRef<Path>) -> anyhow::Result<()> {
    let mut build = Build::from_env();

    // The directory rather than the files in it: adding a page is adding files,
    // and cargo has to notice that too.
    let root = build.track(root);
    let manifest = build.read("Cargo.toml")?;

    let routes = root.join(ROUTES);
    let declaration = fs::read_to_string(&routes)
        .with_context(|| format!("{}: no route declaration", routes.display()))?;
    let generated = navigator::emit(&declaration, adapter(&manifest))
        .with_context(|| format!("{}", routes.display()))?;

    build.write(GENERATED_SLINT, generated.slint)?;
    build.write(GENERATED_RUST, generated.rust)?;

    // The global is reached by name, not by path: it lives in OUT_DIR, which
    // the application cannot spell.
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![build.out_dir().to_path_buf()]);
    let app = root.join(APP);
    slint_build::compile_with_config(&app, config)
        .map_err(|e| anyhow::anyhow!("{}: {e}", app.display()))?;

    Ok(())
}

/// The path the generated Rust reaches `guinea-slint` by, from the
/// application's manifest: the facade's when it depends on the facade, the
/// crate's own otherwise.
fn adapter(manifest: &str) -> &'static str {
    let Ok(manifest) = manifest.parse::<toml::Table>() else {
        return "::guinea::slint";
    };

    let names = |table: Option<&toml::Value>| {
        table
            .and_then(toml::Value::as_table)
            .is_some_and(|dependencies| dependencies.contains_key("guinea"))
    };
    let targets = manifest
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values())
        .any(|target| names(target.get("dependencies")));

    match names(manifest.get("dependencies")) || targets {
        true => "::guinea::slint",
        false => "::guinea_slint",
    }
}

#[cfg(test)]
mod tests {
    use super::adapter;

    #[test]
    fn an_application_on_the_facade_reaches_the_backend_through_it() {
        let manifest = "[dependencies]\nguinea = { version = \"0.30\", features = [\"slint\"] }\n";

        assert_eq!(adapter(manifest), "::guinea::slint");
    }

    #[test]
    fn an_application_on_the_backend_alone_reaches_it_by_its_own_name() {
        let manifest = "[dependencies]\nguinea-slint = \"0.30\"\nguinea-app = \"0.30\"\n";

        assert_eq!(adapter(manifest), "::guinea_slint");
    }
}
