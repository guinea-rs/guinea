//! `#[builds]` - the function that builds a plugin `plugin!` declared.
//!
//! ```
//! # use guinea_app::app::PluginBuilder;
//! # use guinea_macros::{builds, plugin};
//! # #[derive(Clone, Debug, PartialEq)]
//! # pub struct Store(String);
//! #
//! # impl Store {
//! #     fn open(path: &str) -> anyhow::Result<Self> {
//! #         anyhow::ensure!(!path.is_empty(), "a store needs a path");
//! #         Ok(Store(path.to_string()))
//! #     }
//! # }
//! #
//! # pub struct StoreParams {
//! #     path: &'static str,
//! # }
//! #
//! # plugin! {
//! #     pub Settings {
//! #         provides { Store }
//! #     }
//! # }
//! #[builds]
//! fn settings(app: &mut PluginBuilder, params: StoreParams) -> anyhow::Result<Settings> {
//!     Ok(Settings(Store::open(params.path)?))
//! }
//! # fn main() -> anyhow::Result<()> {
//! # let (_host, Settings(store)) =
//! #     guinea_app::app::start("doc", |app| app.plugin::<Settings>(StoreParams { path: "settings.db" }))?;
//! # assert_eq!(store, Store("settings.db".into()));
//! #
//! # let refused = guinea_app::app::start("doc", |app| app.plugin::<Settings>(StoreParams { path: "" }));
//! # assert!(refused.is_err());
//! # Ok(())
//! # }
//! ```
//!
//! As `#[installs]` for a feature: the plugin is what it returns, `Params` is
//! its second argument - `()` when it takes only the builder - and `Exports`
//! is what the manifest listed. A plugin takes no services: what it needs of
//! another plugin comes in its parameters.

use proc_macro::TokenStream as TokenStream1;
use quote::quote;
use syn::{FnArg, ItemFn};

pub fn builds_impl(item: TokenStream1) -> TokenStream1 {
    let function = syn::parse_macro_input!(item as ItemFn);

    let Some(built) = crate::installs::installed(&function) else {
        return syn::Error::new_spanned(
            &function.sig,
            "#[builds] reads the plugin from what the function returns - write \
             `-> anyhow::Result<YourPlugin>`",
        )
        .to_compile_error()
        .into();
    };

    let mut inputs = function.sig.inputs.iter();
    let builder = inputs.next();
    let params = inputs.next();
    if builder.is_none() || inputs.next().is_some() {
        return syn::Error::new_spanned(
            &function.sig.inputs,
            "#[builds] takes the builder and the plugin's parameters - \
             `(app: &mut PluginBuilder, params: YourParams)`, or the builder alone",
        )
        .to_compile_error()
        .into();
    }

    let params = match params {
        Some(FnArg::Typed(params)) => Some(&*params.ty),
        Some(FnArg::Receiver(receiver)) => {
            return syn::Error::new_spanned(receiver, "#[builds] builds from a function, not a method")
                .to_compile_error()
                .into();
        }
        None => None,
    };

    let gc = crate::paths::core();
    let feature = crate::paths::feature();
    let app = crate::paths::app();
    let called = crate::installs::called(&function);
    let (impl_generics, _, where_clause) = function.sig.generics.split_for_impl();

    let (params, call) = match params {
        Some(params) => (quote!(#params), quote!(#called(__guinea_app, __guinea_params))),
        None => (quote!(()), quote!({
            let () = __guinea_params;
            #called(__guinea_app)
        })),
    };

    quote! {
        #function

        impl #impl_generics #app::Plugin for #built #where_clause {
            type Params = #params;
            type Exports = <#built as #feature::Manifest>::Exports;

            fn build(
                __guinea_app: &mut #app::PluginBuilder,
                __guinea_params: Self::Params,
            ) -> #gc::__private::anyhow::Result<Self> {
                #call
            }
        }
    }
    .into()
}
