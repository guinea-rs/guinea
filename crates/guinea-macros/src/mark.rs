use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, spanned::Spanned};


pub(crate) fn derive_mark(input: DeriveInput) -> TokenStream {
    let Data::Enum(data) = &input.data else {
        return syn::Error::new(input.ident.span(), "`Mark` is derived for an enum of names")
            .to_compile_error();
    };

    let mut arms = Vec::with_capacity(data.variants.len());
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return syn::Error::new(variant.fields.span(), "a mark is a name and carries nothing")
                .to_compile_error();
        }

        let ident = &variant.ident;
        let name = ident.to_string();
        arms.push(quote!(Self::#ident => #name));
    }

    let gc = crate::paths::core();
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics #gc::mark::Mark for #name #ty_generics #where_clause {
            fn name(&self) -> &'static str {
                match self {
                    #(#arms,)*
                }
            }
        }
    }
}
