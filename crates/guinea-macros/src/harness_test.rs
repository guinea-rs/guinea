//! `#[guinea::test]` - a test run once per seed, each on a fresh `Harness`.

use proc_macro::TokenStream as TokenStream1;
use quote::quote;
use syn::{ItemFn, LitInt, LitStr, parse_macro_input};

const ITERATIONS: u64 = 64;

pub fn test_impl(attr: TokenStream1, item: TokenStream1) -> TokenStream1 {
    let mut iterations = ITERATIONS;
    let mut exclusive: Option<LitStr> = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("iterations") {
            iterations = meta.value()?.parse::<LitInt>()?.base10_parse()?;
            Ok(())
        } else if meta.path.is_ident("exclusive") {
            exclusive = Some(meta.value()?.parse()?);
            Ok(())
        } else {
            Err(meta.error("expected `iterations = N` or `exclusive = \"key\"`"))
        }
    });
    parse_macro_input!(attr with parser);

    let mut body = parse_macro_input!(item as ItemFn);
    if body.sig.inputs.len() != 1 {
        return syn::Error::new_spanned(
            &body.sig,
            "a #[guinea::test] takes the harness and nothing else: `fn name(h: &mut Harness)`",
        )
        .to_compile_error()
        .into();
    }

    let attrs = std::mem::take(&mut body.attrs);
    let vis = &body.vis;
    let name = &body.sig.ident;
    let app = crate::paths::app();

    let run = match exclusive {
        Some(key) => quote!(#app::check_exclusive(#key, #iterations, #name);),
        None => quote!(#app::check(#iterations, #name);),
    };

    quote! {
        #(#attrs)*
        #[::core::prelude::v1::test]
        #vis fn #name() {
            #body

            #run
        }
    }
    .into()
}
