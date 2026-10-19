//! `feature!` and `plugin!` - a feature's or a plugin's manifest: its name,
//! what it exports and what it feeds, and for a plugin what it provides.
//!
//! ```
//! # use guinea_app::app::{GuineaHost, PluginBuilder};
//! # use guinea_app::app::roots::Root;
//! # use guinea_app::feature::FeatureInitContext;
//! # use guinea_core::feature::Bound;
//! # use guinea_core::feed::Feed;
//! # use guinea_macros::{builds, feature, installs, plugin, reducer};
//! # #[derive(Clone, Default)]
//! # pub struct AgentLinkState {
//! #     pub connected: bool,
//! # }
//! #
//! # #[reducer]
//! # fn agent_link_state(this: &mut AgentLinkState, connected: bool) {
//! #     this.connected = connected;
//! # }
//! #
//! # pub struct AgentReport(pub &'static str);
//! #
//! # #[derive(Clone, Debug, PartialEq)]
//! # pub struct Store(&'static str);
//! feature! {
//!     pub AgentLink {
//!         exports { AgentLinkState }
//!         feeds { AgentReport }
//!     }
//! }
//!
//! plugin! {
//!     pub Settings {
//!         provides { Store }
//!     }
//! }
//! # #[installs]
//! # fn agent_link(cx: &FeatureInitContext) -> anyhow::Result<AgentLink> {
//! #     let link = cx.state::<AgentLinkState>().plain();
//! #     let report = cx.feed::<AgentReport>();
//! #     report.set(AgentReport("idle"));
//! #     Ok(AgentLink(link, report.feed()))
//! # }
//! #
//! # #[builds]
//! # fn settings(_app: &mut PluginBuilder, file: &'static str) -> anyhow::Result<Settings> {
//! #     Ok(Settings(Store(file)))
//! # }
//! #
//! # fn main() -> anyhow::Result<()> {
//! # let host = GuineaHost::new();
//! # let window = Root::under(&host.context());
//! # let AgentLink(link, report) = window.context(window.scope(), 0).install::<AgentLink>(&())?;
//! # let _: (Bound<AgentLinkState>, Feed<AgentReport>) = (link, report);
//! # link.push(true);
//! # assert_eq!(report.get().map(|report| report.0), Some("idle"));
//! #
//! # let page = window.context(window.scope().child(), 1);
//! # assert_eq!(page.read::<AgentLinkState>().map(|state| state.connected), Some(true));
//! #
//! # let (_app, Settings(store)) = guinea_app::app::start("doc", |app| app.plugin::<Settings>("settings.db"))?;
//! # assert_eq!(store, Store("settings.db"));
//! # Ok(())
//! # }
//! ```
//!
//! A list, not a struct: the manifest says what the feature is, and the
//! feature itself is what `#[installs]` builds - a plugin what `#[builds]`
//! builds. The type it makes holds one `Bound` per export, then one `Feed`
//! per feed, then each value provided, in the order listed -
//! `AgentLink(link, report)` - so the only way to have one is to have claimed
//! each thing it exports, opened each feed and made each thing it provides.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Generics, Ident, Token, Type, Visibility, braced};

/// Which manifest is being read: what it may list, and what it is called in
/// an error.
#[derive(Clone, Copy)]
enum Kind {
    Feature,
    Plugin,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Kind::Feature => "feature",
            Kind::Plugin => "plugin",
        }
    }

    fn sections(self) -> &'static str {
        match self {
            Kind::Feature => "`exports` and `feeds`",
            Kind::Plugin => "`exports`, `feeds` and `provides`",
        }
    }
}

struct Manifest {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    generics: Generics,
    exports: Vec<Type>,
    feeds: Vec<Type>,
    provides: Vec<Type>,
}

impl Manifest {
    fn parse(input: ParseStream, kind: Kind) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let name: Ident = input.parse()?;
        let mut generics: Generics = input.parse()?;
        generics.where_clause = input.parse()?;

        let body;
        braced!(body in input);

        let mut exports = Vec::new();
        let mut feeds = Vec::new();
        let mut provides = Vec::new();
        while !body.is_empty() {
            let section: Ident = body.parse()?;
            let into = match (section.to_string().as_str(), kind) {
                ("exports", _) => &mut exports,
                ("feeds", _) => &mut feeds,
                ("provides", Kind::Plugin) => &mut provides,
                _ => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!(
                            "unknown section `{section}`; a {} lists {}",
                            kind.noun(),
                            kind.sections()
                        ),
                    ));
                }
            };

            let listed;
            braced!(listed in body);
            let types: Punctuated<Type, Token![,]> =
                listed.parse_terminated(Type::parse, Token![,])?;
            into.extend(types);
        }

        Ok(Manifest {
            attrs,
            vis,
            name,
            generics,
            exports,
            feeds,
            provides,
        })
    }
}

struct Feature(Manifest);

impl Parse for Feature {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Manifest::parse(input, Kind::Feature).map(Feature)
    }
}

struct Plugin(Manifest);

impl Parse for Plugin {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Manifest::parse(input, Kind::Plugin).map(Plugin)
    }
}

pub fn feature_impl(input: TokenStream1) -> TokenStream1 {
    let Feature(manifest) = syn::parse_macro_input!(input as Feature);
    expand(manifest, Kind::Feature).into()
}

pub fn plugin_impl(input: TokenStream1) -> TokenStream1 {
    let Plugin(manifest) = syn::parse_macro_input!(input as Plugin);
    expand(manifest, Kind::Plugin).into()
}

fn expand(manifest: Manifest, kind: Kind) -> TokenStream {
    let Manifest {
        attrs,
        vis,
        name,
        generics,
        exports,
        feeds,
        provides,
    } = manifest;

    let holds_nothing = exports.is_empty() && feeds.is_empty() && provides.is_empty();
    if holds_nothing && !generics.params.is_empty() {
        return syn::Error::new(
            name.span(),
            format!(
                "a generic {0} holds what it lists, and this one has nothing to hold its \
                 parameters - list what it exports, or drop the parameters",
                kind.noun()
            ),
        )
        .to_compile_error();
    }

    let gc = crate::paths::core();
    let feature = crate::paths::feature();
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let declared = if holds_nothing {
        quote! {
            #(#attrs)*
            #vis struct #name;
        }
    } else {
        let fields = exports
            .iter()
            .map(|export| quote!(#vis #gc::feature::Bound<#export>))
            .chain(feeds.iter().map(|fed| quote!(#vis #gc::feed::Feed<#fed>)))
            .chain(provides.iter().map(|provided| quote!(#vis #provided)));
        quote! {
            #(#attrs)*
            #vis struct #name #generics (#(#fields),*) #where_clause;
        }
    };

    quote! {
        #declared

        impl #impl_generics #feature::Manifest for #name #type_generics #where_clause {
            type Exports = (#(#exports,)*);
        }
    }
}
