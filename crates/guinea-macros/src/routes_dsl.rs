use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Ident, TokenStream};
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote, quote_spanned};

use guinea_route_dsl::{Segment, parse_pattern, same_type, spelled, type_ident};

fn guinea_crate_path() -> proc_macro2::TokenStream {
    match crate_name("guinea") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident)
        }
        Err(_) => quote!(::guinea),
    }
}

/// Where the router's own types live.
///
/// An application usually reaches them through the facade, but a backend crate
/// depends on `guinea-router` directly and has no facade at all - and the
/// generated `RouteChain` impl has to name the same types either way.
/// Where a segment was listed, as the line `ty` was written on: `file!` and
/// friends expand wherever their tokens are spanned.
fn declared_at(ty: &syn::Type) -> TokenStream {
    let gc = crate::handler::guinea_core_crate_path();

    quote_spanned! {syn::spanned::Spanned::span(ty)=>
        #gc::actor::shape::Declared {
            file: ::core::file!(),
            line: ::core::line!(),
            column: ::core::column!(),
            crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
        }
    }
}

fn router_path(guinea: &TokenStream) -> TokenStream {
    match crate_name("guinea-router") {
        Ok(FoundCrate::Itself) => quote!(crate::router),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident::router)
        }
        Err(_) => quote!(#guinea::router),
    }
}

/// Where [`LinkValue`](guinea_router::link::LinkValue) lives. Same reason as
/// [`router_path`].
fn link_path(guinea: &TokenStream) -> TokenStream {
    module_of_router(guinea, "link")
}

/// Where the enter-guard machinery lives.
fn enter_path(guinea: &TokenStream) -> TokenStream {
    module_of_router(guinea, "enter")
}

/// Where the two halves of `restorable` live.
fn restore_path(guinea: &TokenStream) -> TokenStream {
    module_of_router(guinea, "restore")
}

fn module_of_router(guinea: &TokenStream, module: &str) -> TokenStream {
    let module = syn::Ident::new(module, proc_macro2::Span::call_site());
    match crate_name("guinea-router") {
        Ok(FoundCrate::Itself) => quote!(crate::#module),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident::#module)
        }
        Err(_) => quote!(#guinea::#module),
    }
}

/// Where the feature layer's own types live.
///
/// Same reason as [`router_path`]: an application reaches them through the
/// facade, a backend crate depends on `guinea-app` directly and has no facade
/// at all, and the generated `Segment` impls have to name them either way.
fn feature_path(guinea: &TokenStream) -> TokenStream {
    match crate_name("guinea-app") {
        Ok(FoundCrate::Itself) => quote!(crate::feature),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident::feature)
        }
        Err(_) => quote!(#guinea::feature),
    }
}

/// The module a backend keeps `segment_entry`/`layout_entry` in: the parent of
/// the backend type, since it lives next to what it builds for.
fn backend_module(ty: &syn::Type) -> TokenStream {
    let syn::Type::Path(path) = ty else {
        panic!("routes!: backend must be a path, like `guinea_ratatui::Tui`");
    };

    let mut module = path.path.clone();
    module.segments.pop();
    let module: Vec<syn::PathSegment> = module.segments.into_iter().collect();
    if module.is_empty() {
        panic!("routes!: backend needs the module too, as in `guinea_ratatui::Tui`");
    }
    quote!(#(#module)::*)
}

/// `Debug` and `PartialEq` for a route enum, over its identity fields only.
///
/// A route with a `~` field cannot derive either: a channel has no equality
/// and often no `Debug`. Written out, both read the identity and skip the
/// payload - which is the same statement the router makes when it asks whether
/// this is still the same route.
fn identity_impls(enum_ident: &Ident, leaves: &[guinea_route_dsl::Leaf]) -> TokenStream {
    let eq_arms = leaves.iter().map(|leaf| {
        let variant = type_ident(&leaf.ty);
        let kept: Vec<&Ident> = leaf
            .fields
            .iter()
            .filter(|field| field.identity)
            .map(|field| &field.name)
            .collect();
        let theirs: Vec<Ident> = kept.iter().map(|name| format_ident!("__other_{}", name)).collect();

        quote! {
            (
                #enum_ident::#variant { #(#kept,)* .. },
                #enum_ident::#variant { #(#kept: #theirs,)* .. },
            ) => #(#kept == #theirs &&)* true
        }
    });

    let debug_arms = leaves.iter().map(|leaf| {
        let variant = type_ident(&leaf.ty);
        let kept: Vec<&Ident> = leaf
            .fields
            .iter()
            .filter(|field| field.identity)
            .map(|field| &field.name)
            .collect();
        let labels = kept.iter().map(|name| name.to_string());
        let title = variant.to_string();
        let whole = leaf.fields.len() == kept.len();
        let finish = if whole {
            quote!(.finish())
        } else {
            quote!(.finish_non_exhaustive())
        };

        quote! {
            #enum_ident::#variant { #(#kept,)* .. } => f
                .debug_struct(#title)
                #(.field(#labels, #kept))*
                #finish
        }
    });

    quote! {
        impl ::std::cmp::PartialEq for #enum_ident {
            fn eq(&self, other: &Self) -> bool {
                match (self, other) {
                    #(#eq_arms,)*
                    _ => false,
                }
            }
        }

        impl ::std::fmt::Debug for #enum_ident {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    #(#debug_arms),*
                }
            }
        }
    }
}

/// One struct per segment, carrying exactly what it captured.
///
/// Derived when every field has an identity, hand-written when one does not.
/// The hand-written half is the whole point of `~`: a payload that cannot be
/// compared or printed would otherwise force `PartialEq` and `Debug` on a
/// channel.
fn params_struct(name: &Ident, fields: &[guinea_route_dsl::Field]) -> TokenStream {
    let defs = fields.iter().map(|field| {
        let (name, ty) = (&field.name, &field.ty);
        quote! { pub #name: #ty }
    });

    if fields.is_empty() {
        return quote! {
            #[derive(Clone, Debug, Default, PartialEq)]
            pub struct #name {}
        };
    }

    if fields.iter().all(|field| field.identity) {
        return quote! {
            #[derive(Clone, Debug, PartialEq)]
            pub struct #name { #(#defs),* }
        };
    }

    let kept: Vec<&Ident> = fields
        .iter()
        .filter(|field| field.identity)
        .map(|field| &field.name)
        .collect();
    let labels = kept.iter().map(|name| name.to_string());
    let title = name.to_string();

    quote! {
        #[derive(Clone)]
        pub struct #name { #(#defs),* }

        impl ::std::fmt::Debug for #name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct(#title)
                    #(.field(#labels, &self.#kept))*
                    .finish_non_exhaustive()
            }
        }

        impl ::std::cmp::PartialEq for #name {
            /// Never, and deliberately.
            ///
            /// The router's one question about a capture is whether it is
            /// still the same one. Part of this one was thrown away rather
            /// than kept, so the honest answer is that it cannot be told - and
            /// "cannot be told" has to mean "reinstall", because the
            /// alternative is a segment quietly holding a channel its caller
            /// replaced.
            fn eq(&self, _other: &Self) -> bool {
                false
            }
        }
    }
}

fn joined<E: std::fmt::Display>(errors: &[E]) -> String {
    errors
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn routes_impl(input: TokenStream1) -> TokenStream1 {
    let tree = guinea_route_dsl::parse(input.into());
    let enum_ident = tree.name.clone();
    let leaves = tree.leaves();

    // All of them, not the first: a tree whose captures drifted from its
    // fields usually drifted in several places at once, and reporting one per
    // build turns one edit into several.
    let field_errors = guinea_route_dsl::matcher::check_fields(&leaves);
    if !field_errors.is_empty() {
        panic!("{}", joined(&field_errors));
    }

    let guard_errors = guinea_route_dsl::check_guards(&tree);
    if !guard_errors.is_empty() {
        panic!("{}", joined(&guard_errors));
    }

    let layout_errors = guinea_route_dsl::check_layouts(&tree);
    if !layout_errors.is_empty() {
        panic!("{}", joined(&layout_errors));
    }

    let match_tree = match guinea_route_dsl::matcher::build(&leaves) {
        Ok(tree) => tree,
        Err(conflicts) => panic!("{}", joined(&conflicts)),
    };

    let guinea = guinea_crate_path();
    let router = router_path(&guinea);
    let link_mod = link_path(&guinea);
    let enter = enter_path(&guinea);
    let restore_mod = restore_path(&guinea);

    let parse_fn = guinea_route_dsl::matcher::emit_parse(&match_tree, &enum_ident, &link_mod);

    let variant_idents: Vec<Ident> = leaves.iter().map(|l| type_ident(&l.ty)).collect();

    // Identity is generated code rather than a trait bound, which is what lets
    // a route carry something with none. Derived while every field has one,
    // written out when a `~` appears anywhere in the tree - the enum is one
    // item, so one loose field decides for all of it.
    let whole = leaves
        .iter()
        .all(|leaf| leaf.fields.iter().all(|field| field.identity));

    let (enum_derives, enum_impls) = if whole {
        (quote!(#[derive(Debug, PartialEq)]), quote!())
    } else {
        (quote!(), identity_impls(&enum_ident, &leaves))
    };

    let variant_defs = leaves.iter().zip(&variant_idents).map(|(leaf, ident)| {
        let field_defs = leaf.fields.iter().map(|field| {
            let (name, ty) = (&field.name, &field.ty);
            quote! { #name: #ty }
        });
        quote! { #ident { #(#field_defs),* } }
    });

    // One struct per page, carrying exactly what that page captured. This is
    // what reaches `install`, in place of the address it used to dig them out
    // of by position.
    let params_idents: Vec<Ident> = variant_idents
        .iter()
        .map(|ident| format_ident!("{}Params", ident))
        .collect();

    let params_defs = leaves
        .iter()
        .zip(&params_idents)
        .map(|(leaf, params)| params_struct(params, &leaf.fields));

    // A layout is handed what every page under it carries - derived, not
    // declared, so a layout can never ask for something one of its pages does
    // not have. See `RouteTree::layout_params`.
    let layouts = tree.layout_params();
    let layout_params_idents: Vec<Ident> = layouts
        .iter()
        .map(|layout| format_ident!("{}Params", type_ident(&layout.ty)))
        .collect();

    // A layout's own are always identity - `layout_params` leaves `~` fields
    // out, since a comparison is the only reason they exist.
    let layout_params_defs = layouts
        .iter()
        .zip(&layout_params_idents)
        .map(|(layout, params)| params_struct(params, &layout.fields));

    // In chain order: each ancestor layout, then the leaf's own.
    let params_arms = leaves
        .iter()
        .zip(&variant_idents)
        .zip(&params_idents)
        .map(|((leaf, ident), params)| {
            let names: Vec<&Ident> = leaf.fields.iter().map(|field| &field.name).collect();

            let ancestors = leaf.ancestors.iter().map(|ancestor| {
                let position = layouts
                    .iter()
                    .position(|layout| same_type(&layout.ty, ancestor))
                    .expect("every ancestor of a leaf is a layout of this tree");
                let params = &layout_params_idents[position];
                let taken = layouts[position]
                    .fields
                    .iter()
                    .map(|field| {
                        let name = &field.name;
                        quote! { #name: #name.clone() }
                    });

                quote! {
                    ::std::boxed::Box::new(#params { #(#taken),* })
                        as ::std::boxed::Box<dyn ::std::any::Any>
                }
            });

            quote! {
                #enum_ident::#ident { #(#names),* } => vec![
                    #(#ancestors,)*
                    ::std::boxed::Box::new(#params { #(#names: #names.clone()),* })
                        as ::std::boxed::Box<dyn ::std::any::Any>
                ]
            }
        });

    // `restorable` is the one tier that makes the compiler prove something,
    // and the proof is the code itself: a field that does not survive the
    // round trip fails to build. `quote_spanned!` puts that failure on the
    // field's own declaration rather than somewhere inside an expansion.
    let restorable = leaves.iter().any(|leaf| leaf.restorable);

    let save_arms = leaves.iter().zip(&variant_idents).map(|(leaf, ident)| {
        if !leaf.restorable {
            return quote! { #enum_ident::#ident { .. } => None };
        }

        let names: Vec<&Ident> = leaf.fields.iter().map(|field| &field.name).collect();
        let route = ident.to_string();
        let writes = leaf.fields.iter().map(|field| {
            let name = &field.name;
            let key = name.to_string();
            quote_spanned! {name.span()=>
                __saving.field(#key, #name)?;
            }
        });

        quote! {
            #enum_ident::#ident { #(#names),* } => {
                let mut __saving = #restore_mod::Saving::new(#route);
                #(#writes)*
                __saving.finish()
            }
        }
    });

    let restore_arms = leaves
        .iter()
        .zip(&variant_idents)
        .filter(|(leaf, _)| leaf.restorable)
        .map(|(leaf, ident)| {
            let route = ident.to_string();
            let reads = leaf.fields.iter().map(|field| {
                let name = &field.name;
                let key = name.to_string();
                quote_spanned! {name.span()=>
                    #name: __fields.field(#key)?
                }
            });

            quote! {
                #route => Some(#enum_ident::#ident { #(#reads),* })
            }
        });

    let link_arms = leaves.iter().zip(&variant_idents).map(|(leaf, ident)| {
        let Some(link) = &leaf.link else {
            return quote! { #enum_ident::#ident { .. } => None };
        };

        let field_pats: Vec<&Ident> = leaf.fields.iter().map(|field| &field.name).collect();
        let parts = parse_pattern(link).into_iter().map(|seg| match seg {
            Segment::Literal(lit) => quote! { #lit.to_string() },
            Segment::Capture(name) => {
                let field = format_ident!("{}", name);
                quote! { #link_mod::LinkValue::encode(#field) }
            }
        });

        quote! {
            #enum_ident::#ident { #(#field_pats),* } => {
                let parts: Vec<String> = vec![#(#parts),*];
                Some(format!("/{}", parts.join("/")))
            }
        }
    });

    // The whole external surface: what an installer registers and what a
    // committed manifest is diffed against.
    //
    // The capture's type is written as `LinkValue::NAME` rather than as the
    // declaration spelled it, so `type Context = String` renders as `String`
    // and renaming an alias is not a changed address.
    let tree_name = enum_ident.to_string();
    let declared_links = leaves.iter().filter_map(|leaf| {
        let link = leaf.link.as_ref()?;
        let route = type_ident(&leaf.ty).to_string();
        let guard_names = leaf.guards.iter().map(spelled);
        let leaf_restorable = leaf.restorable;

        // In the order the path names them, which is how the address reads.
        let captures = parse_pattern(link).into_iter().filter_map(|seg| {
            let Segment::Capture(name) = seg else {
                return None;
            };
            let ty = leaf
                .fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| &field.ty)
                .expect("check_fields has passed, so every capture has a field");

            quote! {
                #link_mod::Capture {
                    name: #name,
                    ty: <#ty as #link_mod::LinkValue>::NAME,
                }
            }
            .into()
        });

        Some(quote! {
            #link_mod::DeepLink {
                tree: #tree_name,
                route: #route,
                path: #link,
                captures: &[#(#captures),*],
                guards: &[#(#guard_names),*],
                restorable: #leaf_restorable,
            }
        })
    });

    // A name for every route, addressable or not - navigation hooks want to
    // say where the application went, and most routes have no address to say
    // it with.
    let name_arms = variant_idents.iter().map(|ident| {
        let name = ident.to_string();
        quote! { #enum_ident::#ident { .. } => #name }
    });

    // Default to the facade's backend, so an application that has only one
    // never mentions it.
    let (backend_ty, backend_mod) = match &tree.backend {
        Some(ty) => (quote!(#ty), backend_module(ty)),
        None => (quote!(#guinea::Backend), quote!(#guinea::backend)),
    };

    let chain_consts = leaves.iter().zip(&variant_idents).map(|(leaf, ident)| {
        let const_name = format_ident!("__routes_chain_{}_{}", enum_ident, ident);
        let leaf_ty = &leaf.ty;
        let kept = |at: usize| match leaf.keep.get(at) {
            Some(true) => quote!(.kept()),
            _ => quote!(),
        };

        let ancestor_entries = leaf.ancestors.iter().enumerate().map(|(at, ty)| {
            let declared = declared_at(ty);
            let kept = kept(at);
            quote! { #backend_mod::layout_entry::<#ty>().at(#declared) #kept }
        });
        let len = leaf.ancestors.len() + 1;
        let declared = declared_at(leaf_ty);
        let leaf_kept = kept(leaf.ancestors.len());
        quote! {
            #[allow(non_upper_case_globals)]
            const #const_name: [#router::SegmentEntry<#backend_ty>; #len] = [
                #(#ancestor_entries,)*
                #backend_mod::segment_entry::<#leaf_ty>().at(#declared) #leaf_kept,
            ];
        }
    });

    // Where each segment sits, so the compiler can answer what it may read.
    // The macro knows one half - who is above whom - and the author declares
    // the other: `Installs` here, `Exports` on each feature.
    let feature = feature_path(&guinea);
    let above = |ancestors: &[syn::Type]| {
        // Innermost first, as a cons list, so the search reads the way the
        // runtime one used to walk.
        ancestors.iter().rev().fold(quote!(()), |tail, ty| {
            quote! { (#ty, #tail) }
        })
    };

    let mut placements: Vec<TokenStream> = Vec::new();
    for leaf in &leaves {
        let ty = &leaf.ty;
        let above = above(&leaf.ancestors);
        placements.push(quote! {
            impl #feature::Segment for #ty {
                type Installs = <#ty as #backend_mod::Page>::Installs;
                type Above = #above;
            }
        });
    }

    for layout in &layouts {
        let ty = &layout.ty;
        // A layout stands under the same layouts wherever it appears -
        // `check_layouts` refuses a tree where it does not - so the first leaf
        // that sits under it settles what is above it.
        let ancestors = leaves
            .iter()
            .find_map(|leaf| {
                let at = leaf.ancestors.iter().position(|a| same_type(a, ty))?;
                Some(leaf.ancestors[..at].to_vec())
            })
            .unwrap_or_default();
        let above = above(&ancestors);
        placements.push(quote! {
            impl #feature::Segment for #ty {
                type Installs = <#ty as #backend_mod::Layout>::Installs;
                type Above = #above;
            }
        });
    }

    // One `const` per route rather than one shared list: the resolved set
    // differs per leaf, and a `const` keeps the guards where the entries are -
    // in static data, built once, with nothing to allocate at navigation time.
    let guard_consts = leaves.iter().zip(&variant_idents).map(|(leaf, ident)| {
        let const_name = format_ident!("__routes_guards_{}_{}", enum_ident, ident);
        let len = leaf.guards.len();
        let stands = leaf.guards.iter().map(|ty| {
            quote! {
                &const { #enter::Stands::<#ty>(::std::marker::PhantomData) }
            }
        });

        quote! {
            #[allow(non_upper_case_globals)]
            const #const_name: [&'static dyn #enter::EnterGuard; #len] = [#(#stands),*];
        }
    });

    let guard_arms = leaves.iter().zip(&variant_idents).map(|(_leaf, ident)| {
        let const_name = format_ident!("__routes_guards_{}_{}", enum_ident, ident);
        quote! { #enum_ident::#ident { .. } => &#const_name }
    });

    let chain_arms = leaves.iter().zip(&variant_idents).map(|(_leaf, ident)| {
        let const_name = format_ident!("__routes_chain_{}_{}", enum_ident, ident);
        quote! { #enum_ident::#ident { .. } => &#const_name }
    });

    let expanded = quote! {
        #[derive(Clone)]
        #enum_derives
        pub enum #enum_ident {
            #(#variant_defs),*
        }

        #enum_impls

        impl #enum_ident {
            /// The address this route answers to, when it agreed to have one.
            pub fn link(&self) -> Option<String> {
                match self {
                    #(#link_arms),*
                }
            }

            /// Every address the application answers to. What an installer
            /// registers, and what a committed manifest is diffed against.
            pub fn deep_links() -> &'static [#link_mod::DeepLink] {
                &[#(#declared_links),*]
            }

            /// What to call this route in a log or a navigation hook. Present
            /// whether or not the route is addressable.
            pub fn name(&self) -> &'static str {
                match self {
                    #(#name_arms),*
                }
            }

            /// Whether any route in this tree survives a restart.
            ///
            /// The tree's answer, not this route's: an application asks it to
            /// decide whether to keep a saved route at all.
            pub const RESTORABLE: bool = #restorable;

            /// This route as text, when it agreed to survive a restart.
            ///
            /// Where the text goes is the application's business - the router
            /// has no more opinion about storage than it has about drawing.
            pub fn save(&self) -> Option<String> {
                match self {
                    #(#save_arms),*
                }
            }

            /// A route read back from what [`save`](Self::save) wrote.
            ///
            /// `None` for anything unrecognised, which includes what an older
            /// build wrote: a saved session outlives the version that made it,
            /// and a route that no longer exists is an ordinary thing to find
            /// rather than an error. The application falls back to wherever it
            /// starts.
            pub fn restore(text: &str) -> Option<Self> {
                let (__route, __fields) = #restore_mod::Restoring::open(text)?;
                match __route.as_str() {
                    #(#restore_arms,)*
                    _ => None,
                }
            }

            #parse_fn

            /// What each segment of this route's chain captured, in chain
            /// order. The router hands each one to its segment's `install`.
            pub fn params(&self) -> Vec<::std::boxed::Box<dyn ::std::any::Any>> {
                match self {
                    #(#params_arms),*
                }
            }
        }

        #(#params_defs)*

        #(#layout_params_defs)*

        #(#chain_consts)*

        #(#guard_consts)*

        #(#placements)*

        impl #router::RouteChain<#backend_ty> for #enum_ident {
            fn chain(&self) -> &'static [#router::SegmentEntry<#backend_ty>] {
                match self {
                    #(#chain_arms),*
                }
            }

            fn params(&self) -> Vec<::std::boxed::Box<dyn ::std::any::Any>> {
                #enum_ident::params(self)
            }

            fn name(&self) -> &'static str {
                #enum_ident::name(self)
            }

            fn describe(&self) -> String {
                format!("{self:?}")
            }

            fn link(&self) -> Option<String> {
                #enum_ident::link(self)
            }

            fn guards(&self) -> &'static [&'static dyn #enter::EnterGuard] {
                match self {
                    #(#guard_arms),*
                }
            }
        }

    };

    expanded.into()
}
