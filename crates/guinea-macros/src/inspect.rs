//! `#[derive(Inspect)]`: a struct or enum written field by field, as its
//! `Debug` would be, leaving out what is marked `#[inspect(skip)]`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Error, Field, Fields, parse_quote};

pub fn derive_inspect(input: DeriveInput) -> TokenStream {
    match expand(input) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand(mut input: DeriveInput) -> syn::Result<TokenStream> {
    let gc = crate::handler::guinea_core_crate_path();
    let name = &input.ident;

    let body = match &input.data {
        Data::Struct(data) => {
            let shown = name.to_string();
            let (pattern, written) = fields(&gc, &shown, &data.fields)?;
            quote! {
                let #name #pattern = self;
                #written
            }
        }
        Data::Enum(data) => {
            let arms = data
                .variants
                .iter()
                .map(|variant| {
                    let ident = &variant.ident;
                    let (pattern, written) = fields(&gc, &ident.to_string(), &variant.fields)?;
                    Ok(quote!(Self::#ident #pattern => { #written }))
                })
                .collect::<syn::Result<Vec<_>>>()?;
            quote! {
                match self {
                    #(#arms)*
                }
            }
        }
        Data::Union(data) => {
            return Err(Error::new_spanned(
                data.union_token,
                "#[derive(Inspect)] cannot tell which field of a union holds the value",
            ));
        }
    };

    let params: Vec<_> = input.generics.type_params().map(|param| param.ident.clone()).collect();
    let where_clause = input.generics.make_where_clause();
    for param in params {
        where_clause
            .predicates
            .push(parse_quote!(#param: #gc::inspect::Inspect));
    }
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics #gc::inspect::Inspect for #name #ty_generics #where_clause {
            #[allow(unused_variables)]
            fn inspect(&self, __guinea_at: #gc::inspect::Inspector<'_>) {
                #body
            }
        }
    })
}

/// The pattern that binds `fields`, and the calls that write the ones not
/// skipped.
fn fields(gc: &TokenStream, shown: &str, fields: &Fields) -> syn::Result<(TokenStream, TokenStream)> {
    match fields {
        Fields::Unit => Ok((quote!(), quote!(__guinea_at.unit(#shown);))),
        Fields::Named(named) => {
            let mut bound = Vec::new();
            let mut written = Vec::new();
            for (index, field) in named.named.iter().enumerate() {
                let ident = field.ident.as_ref().expect("a named field");
                let local = format_ident!("__guinea_field_{index}");
                bound.push(quote!(#ident: #local));
                if !skipped(field)? {
                    let label = ident.to_string();
                    written.push(quote!(.field(#label, #local as &dyn #gc::inspect::Inspect)));
                }
            }
            Ok((
                quote!({ #(#bound),* }),
                quote!(__guinea_at.record(#shown) #(#written)* .finish();),
            ))
        }
        Fields::Unnamed(unnamed) => {
            let mut bound = Vec::new();
            let mut written = Vec::new();
            for (index, field) in unnamed.unnamed.iter().enumerate() {
                let local = format_ident!("__guinea_field_{index}");
                bound.push(quote!(#local));
                if !skipped(field)? {
                    let label = index.to_string();
                    written.push(quote!(.field(#label, #local as &dyn #gc::inspect::Inspect)));
                }
            }
            Ok((
                quote!(( #(#bound),* )),
                quote!(__guinea_at.tuple(#shown) #(#written)* .finish();),
            ))
        }
    }
}

fn skipped(field: &Field) -> syn::Result<bool> {
    let mut skip = false;
    for attr in field.attrs.iter().filter(|attr| attr.path().is_ident("inspect")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                skip = true;
                return Ok(());
            }
            Err(meta.error("#[inspect(..)] knows only `skip`"))
        })?;
    }
    Ok(skip)
}
