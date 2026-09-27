//! `#[reducer]` - `impl Reducer` written from a function's signature: the
//! state is what the first argument borrows mutably, the update is the second.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Error, FnArg, GenericParam, ItemFn, ReturnType, Type};

pub fn reducer_impl(function: ItemFn) -> TokenStream {
    match expand(&function) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand(function: &ItemFn) -> syn::Result<TokenStream> {
    let signature = &function.sig;

    if let Some(asyncness) = signature.asyncness {
        return Err(Error::new_spanned(
            asyncness,
            "#[reducer] cannot be async: a reducer changes its state and returns",
        ));
    }
    if let ReturnType::Type(_, returned) = &signature.output {
        return Err(Error::new_spanned(
            returned,
            "#[reducer] returns nothing: the state it changes is the result",
        ));
    }

    let inputs: Vec<_> = signature.inputs.iter().collect();
    if inputs.len() != 2 {
        return Err(Error::new(
            signature.inputs.span(),
            "#[reducer] takes the state and what changed it - \
             `fn name(this: &mut State, update: Update)`; a reducer cannot know who asked, \
             so there is no context to take",
        ));
    }

    let state = state(inputs[0])?;
    let update = update(inputs[1])?;

    let gc = crate::handler::guinea_core_crate_path();
    let name = &signature.ident;
    let (impl_generics, _, where_clause) = signature.generics.split_for_impl();

    let turbofish: Vec<_> = signature
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

    Ok(quote! {
        #function

        impl #impl_generics #gc::Reducer for #state #where_clause {
            type Update = #update;

            fn reduce(&mut self, update: #update) {
                #called(self, update)
            }
        }
    })
}

fn state(input: &FnArg) -> syn::Result<Type> {
    let FnArg::Typed(typed) = input else {
        return Err(Error::new_spanned(
            input,
            "#[reducer] is a free function: write `this: &mut State` instead of `self`",
        ));
    };

    match &*typed.ty {
        Type::Reference(reference) if reference.mutability.is_some() => {
            Ok((*reference.elem).clone())
        }
        other => Err(Error::new_spanned(
            other,
            "#[reducer] reads the state from its first argument - write `this: &mut State`",
        )),
    }
}

fn update(input: &FnArg) -> syn::Result<Type> {
    match input {
        FnArg::Typed(typed) => Ok((*typed.ty).clone()),
        FnArg::Receiver(receiver) => Err(Error::new_spanned(
            receiver,
            "#[reducer]'s second argument is what changed the state",
        )),
    }
}
