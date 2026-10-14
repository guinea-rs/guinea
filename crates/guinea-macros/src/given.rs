//! `#[given]`, and what it shares with `#[installs]`: every argument after a
//! function's context is a service the application provided, taken by clone
//! before the body runs.
//!
//! ```ignore
//! impl Plugin for WindowStatePlugin {
//!     const ID: &'static str = "window-state";
//!     type Exports = ();
//!
//!     #[given]
//!     fn build(self, app: &mut PluginBuilder, store: Store) -> anyhow::Result<()> { .. }
//! }
//! ```
//!
//! The method keeps the trait's signature - its receiver and its context -
//! and opens with what the arguments it lost were. Whatever is missing is
//! named all at once, as the error the method returns or, for a method that
//! returns no `Result`, as a panic.
//!
//! `#[page]`, `#[layout]` and `#[segment]` do the same for `install`, after
//! its context and its parameters.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, Pat, PatType, ReturnType, Type};

use crate::paths;

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

/// `let (patterns): (types) = { .. };`, refusing what is missing with
/// `refused` - `?`, or a panic.
pub(crate) fn taking(cx: TokenStream, patterns: &[TokenStream], types: &[&Type], refused: TokenStream) -> TokenStream {
    if types.is_empty() {
        return quote!();
    }

    let feature = paths::feature();
    let uses = probes();
    let probed: Vec<TokenStream> = types.iter().map(|ty| probe(ty)).collect();

    quote! {
        let (#(#patterns,)*): (#(#types,)*) = {
            #uses

            let __guinea_scope = #feature::given::scope(#cx);
            #feature::given::check::<Self>(__guinea_scope, &[#(#probed.need()),*]) #refused;

            (#(#probed.take(__guinea_scope),)*)
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

/// `?` when the function returns a `Result`, a panic when it cannot.
pub(crate) fn refused(output: &ReturnType) -> TokenStream {
    let returns_result = match output {
        ReturnType::Type(_, returned) => match &**returned {
            Type::Path(path) => path.path.segments.last().is_some_and(|last| last.ident == "Result"),
            _ => false,
        },
        ReturnType::Default => false,
    };

    if returns_result {
        quote!(?)
    } else {
        quote!(.unwrap_or_else(|error| ::core::panic!("{error:#}")))
    }
}

pub fn given_impl(item: TokenStream1) -> TokenStream1 {
    let mut method = syn::parse_macro_input!(item as ImplItemFn);
    match expand(&mut method, 1) {
        Ok(()) => quote!(#method).into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// A page's or a layout's `install`: what follows its context and its
/// parameters is taken as `#[given]` takes it.
pub(crate) fn segment_install(item: &mut ItemImpl) -> syn::Result<()> {
    for entry in &mut item.items {
        if let ImplItem::Fn(method) = entry
            && method.sig.ident == "install"
            && method.sig.inputs.len() > 2
        {
            expand(method, 2)?;
        }
    }
    Ok(())
}

/// Keeps the receiver and the first `kept` arguments - the context first -
/// and takes the rest from the context before the body runs.
fn expand(method: &mut ImplItemFn, kept_count: usize) -> syn::Result<()> {
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
    for _ in 1..kept_count {
        if let Some(argument) = arguments.next() {
            kept.push(argument.clone());
        }
    }

    let taken = taken(arguments)?;
    let patterns: Vec<TokenStream> = taken.iter().map(|typed| {
        let pattern = &typed.pat;
        quote!(#pattern)
    }).collect();
    let types: Vec<&Type> = taken.iter().map(|typed| &*typed.ty).collect();

    let name = &named.ident;
    let prelude = taking(quote!(&*#name), &patterns, &types, refused(&method.sig.output));

    let statements = &method.block.stmts;
    let block = quote!({
        #prelude
        #(#statements)*
    });

    method.sig.inputs = kept;
    method.block = syn::parse2(block)?;

    Ok(())
}
