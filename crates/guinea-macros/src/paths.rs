//! Where generated code finds guinea's own crates, from wherever it expands.

use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{FoundCrate, crate_name};
use quote::quote;

/// `module` of `package` (`""` for its root), which the facade re-exports as
/// `guinea::facade`.
///
/// The crate itself is reached by its own name, which each crate gives itself
/// with `extern crate self`, so the path holds in its integration tests too.
/// The facade wins over a direct dependency: the lookup sees dev-dependencies,
/// and an application that names a crate only to test with it would get a
/// path its own build does not have. A direct dependency is the path for a
/// crate with no facade - a backend, or the facade's own tests.
pub(crate) fn resolve(package: &str, module: &str, facade: &str) -> TokenStream {
    let own = crate_name(package);

    if let Ok(FoundCrate::Itself) = own {
        return within(&package.replace('-', "_"), module);
    }
    if let Ok(FoundCrate::Name(guinea)) = crate_name("guinea") {
        return within(&guinea, facade);
    }
    if let Ok(FoundCrate::Name(name)) = own {
        return within(&name, module);
    }

    within("guinea", facade)
}

fn within(root: &str, path: &str) -> TokenStream {
    let root = syn::Ident::new(root, Span::call_site());
    let segments = path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(|segment| syn::Ident::new(segment, Span::call_site()));

    quote!(::#root #(::#segments)*)
}

pub(crate) fn core() -> TokenStream {
    resolve("guinea-core", "", "core")
}

pub(crate) fn feature() -> TokenStream {
    resolve("guinea-app", "feature", "feature")
}

pub(crate) fn app() -> TokenStream {
    resolve("guinea-app", "app", "app")
}

pub(crate) fn router(module: &str) -> TokenStream {
    resolve("guinea-router", module, module)
}

/// The facade's root, for what only the facade names.
pub(crate) fn facade() -> TokenStream {
    match crate_name("guinea") {
        Ok(FoundCrate::Name(name)) => within(&name, ""),
        _ => within("guinea", ""),
    }
}
