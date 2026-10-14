use proc_macro::TokenStream as TokenStream1;
use quote::quote;

pub(crate) fn slot_impl(item: TokenStream1) -> TokenStream1 {
    let item = syn::parse_macro_input!(item as syn::ItemStruct);
    let name = &item.ident;
    let (impl_generics, type_generics, where_clause) = item.generics.split_for_impl();
    let slot = crate::paths::router("slot");

    quote! {
        #item

        impl #impl_generics #slot::Slot for #name #type_generics #where_clause {}
    }
    .into()
}
