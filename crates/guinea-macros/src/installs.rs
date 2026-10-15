//! `#[installs]` - the function that installs a feature `feature!` declared.
//!
//! ```ignore
//! #[installs]
//! fn agent_link(cx: &Install<AgentLinkParams>, store: Store) -> anyhow::Result<AgentLink> {
//!     let (link, _) = cx.state::<AgentLinkState>().driven_by(|port| AgentLinkActor::new(port));
//!     Ok(AgentLink(link))
//! }
//! ```
//!
//! Everything the trait wants is already in the signature: the feature is what
//! it returns, `Params` is what its `Install` carries - `()` for a plain
//! `&FeatureInitContext` - and `Exports` is what the manifest listed. Every
//! argument after the context is a service the application provided, taken
//! by clone before the function runs. So the macro writes
//! `impl Feature` from those, and the function stays a function.
//!
//! A function whose first argument is `&mut FeatureBuilder` installs a
//! feature of the application rather than of a window, and gets
//! `impl AppFeature` instead.
//!
//! Named for what the function does rather than for the trait, because
//! `#[feature]` is ambiguous with Rust's own `feature` attribute wherever it
//! is imported by name.

use proc_macro::TokenStream as TokenStream1;
use quote::quote;
use syn::{FnArg, GenericArgument, GenericParam, ItemFn, PathArguments, ReturnType, Type};

pub fn installs_impl(item: TokenStream1) -> TokenStream1 {
    let function = syn::parse_macro_input!(item as ItemFn);

    let Some(installed) = installed(&function) else {
        return syn::Error::new_spanned(
            &function.sig,
            "#[installs] reads the feature from what the function returns - write \
             `-> anyhow::Result<YourFeature>`",
        )
        .to_compile_error()
        .into();
    };

    let gc = crate::paths::core();
    let feature = crate::paths::feature();

    let taken = match crate::given::taken(function.sig.inputs.iter().skip(1)) {
        Ok(taken) => taken,
        Err(error) => return error.to_compile_error().into(),
    };
    let given: Vec<_> = (0..taken.len())
        .map(|at| quote::format_ident!("__guinea_given_{at}"))
        .collect();
    let named: Vec<_> = given.iter().map(|ident| quote!(#ident)).collect();
    let types: Vec<&Type> = taken.iter().map(|typed| &*typed.ty).collect();

    let declared = quote! {
        const DECLARED: ::core::option::Option<#gc::actor::shape::Declared> =
            ::core::option::Option::Some(#gc::actor::shape::Declared {
                file: ::core::file!(),
                line: ::core::line!(),
                column: ::core::column!(),
                crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
            });
    };
    let (impl_generics, _, where_clause) = function.sig.generics.split_for_impl();
    let called = called(&function);

    if builds_the_application(&function) {
        let app = crate::paths::app();
        let taking = crate::given::taking(quote!(&*__guinea_builder), &named, &types, quote!(?));
        return quote! {
            #function

            impl #impl_generics #app::AppFeature for #installed #where_clause {
                type Exports = <#installed as #feature::Manifest>::Exports;

                #declared

                fn install(
                    __guinea_builder: &mut #app::FeatureBuilder,
                ) -> #gc::__private::anyhow::Result<Self> {
                    #taking
                    #called(__guinea_builder, #(#given),*)
                }
            }
        }
        .into();
    }

    let (params, cx) = match carried(&function) {
        Some(params) => (
            quote!(#params),
            quote!(&#feature::Install::new(__guinea_cx, __guinea_params)),
        ),
        None => (quote!(()), quote!(__guinea_cx)),
    };
    let taking = crate::given::taking(quote!(__guinea_cx), &named, &types, quote!(?));

    quote! {
        #function

        impl #impl_generics #feature::Feature for #installed #where_clause {
            type Params = #params;
            type Exports = <#installed as #feature::Manifest>::Exports;

            #declared

            fn install(
                __guinea_cx: &#feature::FeatureInitContext,
                __guinea_params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<Self> {
                let _ = __guinea_params;
                #taking
                #called(#cx, #(#given),*)
            }
        }
    }
    .into()
}

/// The function called with its own generic parameters, turbofished.
pub(crate) fn called(function: &ItemFn) -> proc_macro2::TokenStream {
    let name = &function.sig.ident;
    let turbofish: Vec<_> = function
        .sig
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Type(ty) => Some(&ty.ident),
            GenericParam::Const(constant) => Some(&constant.ident),
            GenericParam::Lifetime(_) => None,
        })
        .collect();

    if turbofish.is_empty() {
        quote!(#name)
    } else {
        quote!(#name::<#(#turbofish),*>)
    }
}

/// The `P` of a first argument `&Install<P>` - `()` for a bare `&Install` -
/// or `None` for any other context.
fn carried(function: &ItemFn) -> Option<Type> {
    let Some(FnArg::Typed(first)) = function.sig.inputs.first() else {
        return None;
    };
    let Type::Reference(reference) = &*first.ty else {
        return None;
    };
    let Type::Path(path) = &*reference.elem else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "Install" {
        return None;
    }

    let carried = match &last.arguments {
        PathArguments::AngleBracketed(arguments) => arguments.args.iter().find_map(|argument| match argument {
            GenericArgument::Type(ty) => Some(ty.clone()),
            _ => None,
        }),
        _ => None,
    };
    Some(carried.unwrap_or_else(|| syn::parse_quote!(())))
}

/// Whether the first argument is `&mut FeatureBuilder` - the application's
/// builder rather than a window feature's context.
fn builds_the_application(function: &ItemFn) -> bool {
    let Some(FnArg::Typed(first)) = function.sig.inputs.first() else {
        return false;
    };
    let Type::Reference(reference) = &*first.ty else {
        return false;
    };
    let Type::Path(path) = &*reference.elem else {
        return false;
    };

    reference.mutability.is_some()
        && path
            .path
            .segments
            .last()
            .is_some_and(|last| last.ident == "FeatureBuilder")
}

/// The `T` in the function's `-> …Result<T>`.
pub(crate) fn installed(function: &ItemFn) -> Option<Type> {
    let ReturnType::Type(_, returned) = &function.sig.output else {
        return None;
    };
    let Type::Path(path) = &**returned else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };

    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    })
}
