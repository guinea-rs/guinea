use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::handler::guinea_core_crate_path;

pub(crate) fn derive_request(input: DeriveInput) -> TokenStream {
    let mut reply: Option<syn::Type> = None;

    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("request")) {
        let parsed = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("reply") {
                if reply.is_some() {
                    return Err(meta.error("a request has one reply type"));
                }
                reply = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("`request` takes `reply = Type`"))
            }
        });
        if let Err(error) = parsed {
            return error.to_compile_error();
        }
    }

    let Some(reply) = reply else {
        return syn::Error::new(
            input.ident.span(),
            "say what answers it: `#[request(reply = Type)]`",
        )
        .to_compile_error();
    };

    let gc = guinea_core_crate_path();
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics #gc::actor::event_bus::rpc::RpcCall for #name #ty_generics #where_clause {
            type Response = #reply;
        }
    }
}
