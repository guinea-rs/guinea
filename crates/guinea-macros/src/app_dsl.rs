//! `app!` - the application's manifest: its name and what its pages read.
//!
//! ```ignore
//! app! {
//!     pub App {
//!         installs { ActivityFeature, L10nPlugin<Strings>, #[cfg(debug_assertions)] Overlay }
//!     }
//! }
//! ```
//!
//! The counterpart of `feature!`. The type it makes holds one `Installed` per
//! line, in the order listed, so the only way to have one is to have installed
//! each of them - `#[installs]` writes the function that does. It is also the
//! top segment of every route tree that names it with `app = ..`, with the
//! list as its `Installs`.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Ident, Meta, Token, Type, Visibility, braced};

/// `guinea_app::app`, or the facade's `guinea::app`.
pub(crate) fn app_path() -> TokenStream {
    match crate_name("guinea-app") {
        Ok(FoundCrate::Itself) => return quote!(crate::app),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            return quote!(::#ident::app);
        }
        Err(_) => {}
    }

    match crate_name("guinea") {
        Ok(FoundCrate::Itself) => quote!(crate::app),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!(::#ident::app)
        }
        Err(_) => quote!(::guinea::app),
    }
}

struct Line {
    cfg: Vec<TokenStream>,
    ty: Type,
}

struct Manifest {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    installs: Vec<Line>,
}

impl Parse for Line {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut cfg = Vec::new();
        for attribute in input.call(Attribute::parse_outer)? {
            match &attribute.meta {
                Meta::List(list) if list.path.is_ident("cfg") => cfg.push(list.tokens.clone()),
                _ => {
                    return Err(syn::Error::new_spanned(
                        attribute,
                        "only `#[cfg(..)]` goes over a line of `installs`",
                    ));
                }
            }
        }

        Ok(Line {
            cfg,
            ty: input.parse()?,
        })
    }
}

impl Parse for Manifest {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let name: Ident = input.parse()?;
        if input.peek(Token![<]) {
            return Err(input.error("an application is one type, not a generic one"));
        }

        let body;
        braced!(body in input);

        let mut installs = Vec::new();
        while !body.is_empty() {
            let section: Ident = body.parse()?;
            if section != "installs" {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section `{section}`; an application lists `installs`"),
                ));
            }

            let listed;
            braced!(listed in body);
            let lines = listed.parse_terminated(Line::parse, Token![,])?;
            installs.extend(lines);
        }

        Ok(Manifest {
            attrs,
            vis,
            name,
            installs,
        })
    }
}

pub fn app_impl(input: TokenStream1) -> TokenStream1 {
    let manifest = syn::parse_macro_input!(input as Manifest);
    expand(manifest).into()
}

fn expand(manifest: Manifest) -> TokenStream {
    let Manifest {
        attrs,
        vis,
        name,
        installs,
    } = manifest;

    let app = app_path();
    let feature = crate::segment::context_path();

    let fields = installs.iter().map(|line| {
        let ty = &line.ty;
        let cfg = &line.cfg;
        quote! {
            #(#[cfg(#cfg)])*
            #vis #app::Installed<#ty>
        }
    });

    let declared = if installs.is_empty() {
        quote! {
            #(#attrs)*
            #vis struct #name;
        }
    } else {
        quote! {
            #(#attrs)*
            #vis struct #name(#(#fields),*);
        }
    };

    let mut aliases = Vec::new();
    let listed = installs.iter().enumerate().map(|(at, line)| {
        let ty = &line.ty;
        if line.cfg.is_empty() {
            return quote!(#ty);
        }

        let alias = format_ident!("__{}Installs{}", name, at);
        let cfg = &line.cfg;
        aliases.push(quote! {
            #[cfg(all(#(#cfg),*))]
            #[allow(non_camel_case_types)]
            type #alias = #ty;

            #[cfg(not(all(#(#cfg),*)))]
            #[allow(non_camel_case_types)]
            type #alias = ();
        });
        quote!(#alias)
    });
    let listed: Vec<TokenStream> = listed.collect();

    quote! {
        #declared

        #(#aliases)*

        impl #feature::Segment for #name {
            type Installs = (#(#listed,)*);
            type Above = ();
        }
    }
}
