//! `#[page]` and `#[layout]`, for the Elm backends and the immediate ones.
//!
//! Two jobs: write down what the author did not, and the hidden `Mountable`
//! a layout's `cx.mount::<S>` takes - one trait for pages and layouts both,
//! which blanket impls over the two could not give without overlapping.
//!
//! Stable Rust has no defaults for associated types, so a page that captures
//! nothing and has no messages of its own still has to say `type Params = ();`
//! and `type Message = ..;` and write the empty `update` that goes with them.
//! That is three lines saying nothing, and nothing but a macro can remove
//! them. The same goes for an `install` that mounts the default node and
//! installs nothing.
//!
//! Deliberately *only* that. Anything a type can check, a type should check -
//! a macro that derives a list from a body is a second source of truth that
//! looks like one source. What a node watches needs no such help: the
//! translation is named at the `cx.on(..)` call and nowhere else, so one that
//! is never registered is a function nobody calls, and the dead-code lint
//! already says so.

use proc_macro::TokenStream as TokenStream1;
use quote::quote;
use syn::{ImplItem, ItemImpl, parse_quote};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Page,
    Layout,
}

/// Whether a node has messages and an `update` - the Elm backends - or draws
/// and changes itself in one call, as an immediate-mode one does.
#[derive(Clone, Copy, PartialEq)]
pub enum Model {
    Elm,
    Immediate,
}

pub fn node_impl(item: TokenStream1, kind: Kind, package: &str, facade: &str, model: Model) -> TokenStream1 {
    let mut item = syn::parse_macro_input!(item as ItemImpl);
    if let Err(error) = crate::given::segment_install(&mut item) {
        return error.to_compile_error().into();
    }
    let adapter = crate::paths::resolve(package, "", facade);

    let declared_type = |name: &str| {
        item.items
            .iter()
            .any(|entry| matches!(entry, ImplItem::Type(ty) if ty.ident == name))
    };
    let declared_fn = |name: &str| {
        item.items
            .iter()
            .any(|entry| matches!(entry, ImplItem::Fn(f) if f.sig.ident == name))
    };

    let has_params = declared_type("Params");
    let has_message = declared_type("Message");
    let has_update = declared_fn("update");
    let has_install = declared_fn("install");

    let gc = crate::paths::core();
    item.items.push(parse_quote! {
        const DECLARED: ::core::option::Option<#gc::actor::shape::Declared> =
            ::core::option::Option::Some(#gc::actor::shape::Declared {
                file: ::core::file!(),
                line: ::core::line!(),
                column: ::core::column!(),
                crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
            });
    });

    if !has_params {
        item.items.push(parse_quote!(
            type Params = ();
        ));
    }

    if !has_install {
        item.items.push(parse_quote! {
            fn install(
                _ctx: &#adapter::FeatureInitContext,
                _params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<Self> {
                ::std::result::Result::Ok(::core::default::Default::default())
            }
        });
    }

    if model == Model::Elm && !has_message {
        item.items.push(parse_quote!(
            type Message = ::core::convert::Infallible;
        ));

        // Only alongside the defaulted message: a node that named its own and
        // then left `update` out has forgotten something, and the missing
        // trait item is a better error than an `update` that swallows.
        if !has_update {
            item.items.push(parse_quote! {
                fn update(
                    &mut self,
                    message: Self::Message,
                    _cx: &mut #adapter::UpdateCx<'_, Self>,
                ) {
                    match message {}
                }
            });
        }
    }

    let (impl_generics, _, where_clause) = item.generics.split_for_impl();
    let node = &item.self_ty;
    let (trait_name, install) = match kind {
        Kind::Page => (quote!(Page), quote!(install_page)),
        Kind::Layout => (quote!(Layout), quote!(install_layout)),
    };

    let message = match model {
        Model::Elm => quote!(type Message = <Self as #adapter::#trait_name>::Message;),
        Model::Immediate => quote!(),
    };

    let mountable = quote! {
        impl #impl_generics #adapter::Mountable for #node #where_clause {
            type Params = <Self as #adapter::#trait_name>::Params;
            #message

            fn install(
                cx: &#adapter::FeatureInitContext,
                params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<()> {
                #adapter::#install::<Self>(cx, params)
            }
        }
    };

    quote!(#item #mountable).into()
}
