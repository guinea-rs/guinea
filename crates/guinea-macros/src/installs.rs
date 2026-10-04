//! `#[installs]` - the function that installs a feature `feature!` declared.
//!
//! ```ignore
//! #[installs]
//! fn agent_link(cx: &FeatureInitContext, params: &AgentLinkParams) -> anyhow::Result<AgentLink> {
//!     let (link, _) = cx.state::<AgentLinkState>().driven_by(|port| AgentLinkActor::new(port));
//!     Ok(AgentLink(link))
//! }
//! ```
//!
//! Everything the trait wants is already in the signature: the feature is what
//! it returns, `Params` is what its second argument points at - `()` when
//! there is none - and `Exports` is what the manifest listed. So the macro
//! writes `impl Feature` from those, and the function stays a function.
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

    if builds_the_application(&function) {
        let app = crate::app_dsl::app_path();
        return quote! {
            #function

            impl #app::Application for #installed {
                fn install(
                    __builder: &mut #app::FeatureBuilder,
                ) -> #gc::__private::anyhow::Result<Self> {
                    #name(__builder)
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
            _ => None,
        })
        .collect();
    let called = if turbofish.is_empty() {
        quote!(#name)
    } else {
        quote!(#name::<#(#turbofish),*>)
    };

    let (params, call) = match function.sig.inputs.iter().nth(1) {
        Some(FnArg::Typed(second)) => {
            let params = match &*second.ty {
                Type::Reference(reference) => (*reference.elem).clone(),
                other => other.clone(),
            };
            (quote!(#params), quote!(#called(cx, params)))
        }
        _ => (quote!(()), quote!(#called(cx))),
    };

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
                cx: &#feature::FeatureInitContext,
                params: &Self::Params,
            ) -> #gc::__private::anyhow::Result<Self> {
                let _ = params;
                #call
            }
        }
    }
    .into()
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
