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
//! by clone before the function runs; see `#[given]`. So the macro writes
//! `impl Feature` from those, and the function stays a function.
//!
//! A function whose first argument is `&mut FeatureBuilder` installs the
//! application `app!` declared, and gets `impl Application` instead.
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

    let gc = crate::handler::guinea_core_crate_path();
    let feature = crate::segment::context_path();
    let name = &function.sig.ident;

    let taken = match crate::given::taken(function.sig.inputs.iter().skip(1)) {
        Ok(taken) => taken,
        Err(error) => return error.to_compile_error().into(),
    };
    let given: Vec<_> = (0..taken.len())
        .map(|at| quote::format_ident!("__guinea_given_{at}"))
        .collect();
    let named: Vec<_> = given.iter().map(|ident| quote!(#ident)).collect();
    let types: Vec<&Type> = taken.iter().map(|typed| &*typed.ty).collect();

    if builds_the_application(&function) {
        let app = crate::app_dsl::app_path();
        let taking = crate::given::taking(quote!(&*__builder), &named, &types, quote!(?));
        return quote! {
            #function

            impl #app::Application for #installed {
                fn install(
                    __builder: &mut #app::FeatureBuilder,
                ) -> #gc::__private::anyhow::Result<Self> {
                    #taking
                    #name(__builder, #(#given),*)
                }
            }
        }
        .into();
    }
    let (impl_generics, _, where_clause) = function.sig.generics.split_for_impl();

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
    let called = if turbofish.is_empty() {
        quote!(#name)
    } else {
        quote!(#name::<#(#turbofish),*>)
    };

    let (params, cx) = match carried(&function) {
        Some(params) => (
            quote!(#params),
            quote!(&#feature::Install::new(__guinea_cx, __guinea_params)),
        ),
        None => (quote!(()), quote!(__guinea_cx)),
    };
    let taking = crate::given::taking(quote!(__guinea_cx), &named, &types, quote!(?));
    let needs = (!types.is_empty()).then(|| {
        let uses = crate::given::probes();
        let probed = types.iter().map(|ty| crate::given::probe(ty));
        quote! {
            fn needs(__guinea_into: &mut ::std::vec::Vec<#feature::given::Need>) {
                #uses

                __guinea_into.extend([#(#probed.need()),*].into_iter().flatten());
            }
        }
    });

    quote! {
        #function

        impl #impl_generics #feature::Feature for #installed #where_clause {
            type Params = #params;
            type Exports = <#installed as #feature::Manifest>::Exports;

            const DECLARED: ::core::option::Option<#gc::actor::shape::Declared> =
                ::core::option::Option::Some(#gc::actor::shape::Declared {
                    file: ::core::file!(),
                    line: ::core::line!(),
                    column: ::core::column!(),
                    crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
                });

            fn install(
                __guinea_cx: &#feature::FeatureInitContext,
                __guinea_params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<Self> {
                let _ = __guinea_params;
                #taking
                #called(#cx, #(#given),*)
            }

            #needs
        }
    }
    .into()
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
/// builder rather than a feature's context.
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
fn installed(function: &ItemFn) -> Option<Type> {
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
