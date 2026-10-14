use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;


pub(crate) fn derive_remote(input: DeriveInput) -> TokenStream {
    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &input.generics,
            "a remote action or event is one type, named by its name - it cannot be generic",
        )
        .to_compile_error();
    }

    let (mut action, mut event) = (false, false);
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("remote")) {
        let parsed = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("action") {
                action = true;
                Ok(())
            } else if meta.path.is_ident("event") {
                event = true;
                Ok(())
            } else {
                Err(meta.error("`remote` takes `action`, `event`, or both"))
            }
        });
        if let Err(error) = parsed {
            return error.to_compile_error();
        }
    }

    if !action && !event {
        return syn::Error::new(
            input.ident.span(),
            "say what a tool may do with it: `#[remote(action)]`, `#[remote(event)]`, or both",
        )
        .to_compile_error();
    }

    let gc = crate::paths::core();
    let name = &input.ident;
    let called = name.to_string();

    let action = action.then(|| {
        quote! {
            #gc::__private::inventory::submit! {
                #gc::remote::RemoteAction {
                    name: #called,
                    path: ::core::concat!(::core::module_path!(), "::", #called),
                    answered_by: #gc::remote::answered_by::<#name>,
                    emit: #gc::remote::emit_json::<#name>,
                }
            }
        }
    });
    let event = event.then(|| {
        quote! {
            #gc::__private::inventory::submit! {
                #gc::remote::RemoteEvent {
                    name: #called,
                    path: ::core::concat!(::core::module_path!(), "::", #called),
                    publish: #gc::remote::publish_json::<#name>,
                }
            }
        }
    });

    quote! {
        #action
        #event
    }
}
