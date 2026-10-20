//! What `#[installs]`, `#[page]` and `#[layout]` share: every argument after
//! a function's context - and, for a page's or a layout's `install`, its
//! parameters - is a service the application provided, taken by clone before
//! the body runs.
//!
//! The method keeps the trait's signature and opens with what the arguments
//! it lost were. Unavailable dependencies are composition errors and panic
//! before the body runs, naming all missing inputs and declared providers.

use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, Pat, PatType, Type};

use crate::paths;

pub(crate) fn descriptor(owner: TokenStream, kind: &str, types: &[&Type], outputs: TokenStream) -> TokenStream {
    let feature = paths::feature();
    let gc = paths::core();
    let uses = probes();
    let inputs = types.iter().map(|ty| probe(ty));
    quote! {
        {
            #uses
            #feature::declarations::Declaration::of::<#owner>(
                #kind,
                #gc::actor::shape::Declared {
                    file: ::core::file!(), line: ::core::line!(),
                    column: ::core::column!(), crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
                },
                ::std::vec![#(#inputs.input(),)*], #outputs,
            )
        }
    }
}

pub(crate) fn registration(function: TokenStream, generics: &syn::Generics, signature: TokenStream) -> TokenStream {
    let gc = paths::core();
    let feature = paths::feature();
    if !generics.params.is_empty() {
        return quote! {
            #gc::__private::inventory::submit! {
                #feature::declarations::TemplateRegistration {
                    target: ::core::stringify!(#function),
                    signature: ::core::stringify!(#signature),
                    source: #gc::actor::shape::Declared {
                        file: ::core::file!(), line: ::core::line!(),
                        column: ::core::column!(), crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
                    },
                }
            }
        };
    }
    quote! {
        #gc::__private::inventory::submit! {
            #feature::declarations::Registration(#function)
        }
    }
}

/// The services among `arguments`: each by value, since it is handed out by
/// clone.
pub(crate) fn taken<'a>(arguments: impl Iterator<Item = &'a FnArg>) -> syn::Result<Vec<&'a PatType>> {
    arguments
        .map(|argument| match argument {
            FnArg::Typed(typed) => match &*typed.ty {
                Type::Reference(reference) => Err(syn::Error::new_spanned(
                    reference,
                    "a service is taken by value - it is handed out by clone; what a segment \
                     hands a feature is read through `cx: &Install<P>`",
                )),
                _ => Ok(typed),
            },
            FnArg::Receiver(receiver) => Err(syn::Error::new_spanned(
                receiver,
                "the receiver comes before the context",
            )),
        })
        .collect()
}

/// Resolves the declared arguments after checking their availability.
pub(crate) fn taking(cx: TokenStream, patterns: &[TokenStream], types: &[&Type]) -> TokenStream {
    if types.is_empty() {
        return quote!();
    }

    let feature = paths::feature();
    let gc = paths::core();
    let uses = probes();
    let probed: Vec<TokenStream> = types.iter().map(|ty| probe(ty)).collect();

    quote! {
        let (#(#patterns,)*): (#(#types,)*) = {
            #uses

            let __guinea_lookup = #feature::given::lookup(#cx);
            #feature::given::check_inputs::<Self>(__guinea_lookup, &[#(#probed.input()),*],
                #gc::actor::shape::Declared {
                    file: ::core::file!(), line: ::core::line!(),
                    column: ::core::column!(), crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
                });

            (#(#probed.take(__guinea_lookup),)*)
        };
    }
}

/// The traits [`probe`] picks among, in scope for it.
pub(crate) fn probes() -> TokenStream {
    let feature = paths::feature();
    quote! {
        #[allow(unused_imports)]
        use #feature::given::{Bare as _, Once as _, Twice as _};
    }
}

/// What an argument of type `ty` is taken through.
pub(crate) fn probe(ty: &Type) -> TokenStream {
    let feature = paths::feature();
    quote!((&&#feature::given::Given::<#ty>::new()))
}


/// A page's or a layout's `install`: what follows its context is taken as a
/// service.
pub(crate) fn segment_install(item: &mut ItemImpl, kind: &str) -> syn::Result<TokenStream> {
    let feature = paths::feature();
    let mut types = Vec::new();
    let mut signature = quote!();
    for entry in &mut item.items {
        if let ImplItem::Fn(method) = entry
            && method.sig.ident == "install"
            && method.sig.inputs.len() > 1
        {
            let original = &method.sig;
            signature = quote!(#original);
            types = taken(method.sig.inputs.iter().skip(1))?.iter().map(|typed| (*typed.ty).clone()).collect();
            expand(method)?;
        }
    }
    let owner = &item.self_ty;
    let method = quote::format_ident!("__guinea_{}_declaration", kind.to_lowercase());
    let types: Vec<_> = types.iter().collect();
    let descriptor = descriptor(quote!(Self), kind, &types, quote!(::std::vec::Vec::new()));
    let (impl_generics, _, where_clause) = item.generics.split_for_impl();
    let registration = registration(quote!(#owner::#method), &item.generics, signature);
    Ok(quote! {
        impl #impl_generics #owner #where_clause {
            #[doc(hidden)]
            fn #method() -> #feature::declarations::Declaration { #descriptor }
        }
        #registration
    })
}

/// Keeps the receiver and the context, and takes the rest from the context
/// before the body runs.
fn expand(method: &mut ImplItemFn) -> syn::Result<()> {
    let mut arguments = method.sig.inputs.iter();
    let mut kept: Punctuated<FnArg, syn::Token![,]> = Punctuated::new();

    let mut context = arguments.next();
    if let Some(receiver @ FnArg::Receiver(_)) = context {
        kept.push(receiver.clone());
        context = arguments.next();
    }

    let Some(FnArg::Typed(context)) = context else {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "the services are reached through the context, the first argument after the \
             receiver",
        ));
    };
    let Pat::Ident(named) = &*context.pat else {
        return Err(syn::Error::new_spanned(
            &context.pat,
            "the services are reached through the context - give it a name",
        ));
    };
    kept.push(FnArg::Typed(context.clone()));

    let taken = taken(arguments)?;
    let patterns: Vec<TokenStream> = taken.iter().map(|typed| {
        let pattern = &typed.pat;
        quote!(#pattern)
    }).collect();
    let types: Vec<&Type> = taken.iter().map(|typed| &*typed.ty).collect();

    let name = &named.ident;
    let prelude = taking(quote!(&*#name), &patterns, &types);

    let statements = &method.block.stmts;
    let block = quote!({
        #prelude
        #(#statements)*
    });

    method.sig.inputs = kept;
    method.block = syn::parse2(block)?;

    Ok(())
}
