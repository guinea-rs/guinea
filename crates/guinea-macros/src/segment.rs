//! `#[segment]` - what a page or layout that installs nothing did not write.
//!
//! Declaring what a segment installs is worth its line when there is something
//! to declare. When there is not, `type Installs = ();` and an `install` that
//! returns it are four lines of nothing, and stable Rust has no conditional
//! default body to remove them - a default returning `()` would have to hold
//! for the segments that return a feature too.
//!
//! So the macro writes them, and only them. It is backend-agnostic on purpose,
//! save for one word: a backend whose segment is a node of its own (ratatui,
//! eframe) mounts it in `install`, so `#[segment(node)]` returns the default
//! node beside the nothing installed. Anything else backend-specific belongs
//! in that backend's own attribute.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::quote;
use syn::{ImplItem, ItemImpl, parse_quote};

/// Where `FeatureInitContext` lives, from wherever this is being expanded.
pub(crate) fn context_path() -> TokenStream {
    match crate_name("guinea-app") {
        Ok(FoundCrate::Itself) => return quote!(crate::feature),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            return quote!(::#ident::feature);
        }
        Err(_) => {}
    }

    match crate_name("guinea") {
        Ok(FoundCrate::Itself) => quote!(crate::feature),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident::feature)
        }
        Err(_) => quote!(::guinea::feature),
    }
}

pub fn segment_impl(attr: TokenStream1, item: TokenStream1) -> TokenStream1 {
    let node = match syn::parse::<Option<syn::Ident>>(attr) {
        Ok(None) => false,
        Ok(Some(word)) if word == "node" => true,
        Ok(Some(word)) => {
            return syn::Error::new_spanned(word, "#[segment] takes nothing, or `node`")
                .to_compile_error()
                .into();
        }
        Err(error) => return error.to_compile_error().into(),
    };

    let mut item = syn::parse_macro_input!(item as ItemImpl);
    if let Err(error) = crate::given::segment_install(&mut item) {
        return error.to_compile_error().into();
    }
    let context = context_path();

    let declares_installs = item
        .items
        .iter()
        .any(|entry| matches!(entry, ImplItem::Type(ty) if ty.ident == "Installs"));
    let declares_install = item
        .items
        .iter()
        .any(|entry| matches!(entry, ImplItem::Fn(f) if f.sig.ident == "install"));

    let gc = crate::handler::guinea_core_crate_path();
    let declared: ImplItem = parse_quote! {
        const DECLARED: ::core::option::Option<#gc::actor::shape::Declared> =
            ::core::option::Option::Some(#gc::actor::shape::Declared {
                file: ::core::file!(),
                line: ::core::line!(),
                column: ::core::column!(),
                crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
            });
    };
    item.items.push(declared);

    if declares_installs {
        return quote!(#item).into();
    }

    item.items.push(parse_quote!(
        type Installs = ();
    ));

    // Only alongside the defaulted list: a segment that named what it installs
    // and then left `install` out has forgotten to install it, and the missing
    // trait item says so more clearly than anything written here could.
    if !declares_install && node {
        item.items.push(parse_quote! {
            fn install(
                _ctx: &#context::FeatureInitContext,
                _params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<(Self, ())> {
                ::std::result::Result::Ok((::core::default::Default::default(), ()))
            }
        });
    } else if !declares_install {
        item.items.push(parse_quote! {
            fn install(
                _ctx: &#context::FeatureInitContext,
                _params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<()> {
                ::std::result::Result::Ok(())
            }
        });
    }

    quote!(#item).into()
}
