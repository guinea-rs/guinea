//! `feature!` - a feature's manifest: its name, what it exports and what it
//! feeds.
//!
//! ```ignore
//! feature! {
//!     pub AgentLink {
//!         exports { AgentLinkState }
//!         feeds { AgentReport }
//!     }
//! }
//! ```
//!
//! A list, not a struct: the manifest says what the feature is, and the
//! feature itself is what `#[installs]` builds. The type it makes holds one
//! `Bound` per export, then one `Feed` per feed, in the order listed -
//! `AgentLink(link, report)` - so the only way to have one is to have claimed
//! each thing it exports and opened each feed.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Generics, Ident, Token, Type, Visibility, braced};

struct Manifest {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    generics: Generics,
    exports: Vec<Type>,
    feeds: Vec<Type>,
}

impl Parse for Manifest {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let name: Ident = input.parse()?;
        let mut generics: Generics = input.parse()?;
        generics.where_clause = input.parse()?;

        let body;
        braced!(body in input);

        let mut exports = Vec::new();
        let mut feeds = Vec::new();
        while !body.is_empty() {
            let section: Ident = body.parse()?;
            let into = match section.to_string().as_str() {
                "exports" => &mut exports,
                "feeds" => &mut feeds,
                _ => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!("unknown section `{section}`; a feature lists `exports` and `feeds`"),
                    ));
                }
            };

            let listed;
            braced!(listed in body);
            let types: Punctuated<Type, Token![,]> =
                listed.parse_terminated(Type::parse, Token![,])?;
            into.extend(types);
        }

        Ok(Manifest {
            attrs,
            vis,
            name,
            generics,
            exports,
            feeds,
        })
    }
}

pub fn feature_impl(input: TokenStream1) -> TokenStream1 {
    let manifest = syn::parse_macro_input!(input as Manifest);
    expand(manifest).into()
}

fn expand(manifest: Manifest) -> TokenStream {
    let Manifest {
        attrs,
        vis,
        name,
        generics,
        exports,
        feeds,
    } = manifest;

    if exports.is_empty() && feeds.is_empty() && !generics.params.is_empty() {
        return syn::Error::new(
            name.span(),
            "a generic feature holds what it exports and feeds, and this one has nothing to hold \
             its parameters - list what it exports, or drop the parameters",
        )
        .to_compile_error();
    }

    let gc = crate::paths::core();
    let feature = crate::paths::feature();
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let declared = if exports.is_empty() && feeds.is_empty() {
        quote! {
            #(#attrs)*
            #vis struct #name;
        }
    } else {
        let fields = exports
            .iter()
            .map(|export| quote!(#vis #gc::feature::Bound<#export>))
            .chain(feeds.iter().map(|fed| quote!(#vis #gc::feed::Feed<#fed>)));
        quote! {
            #(#attrs)*
            #vis struct #name #generics (#(#fields),*) #where_clause;
        }
    };

    quote! {
        #declared

        impl #impl_generics #feature::Manifest for #name #type_generics #where_clause {
            type Exports = (#(#exports,)*);
        }
    }
}
