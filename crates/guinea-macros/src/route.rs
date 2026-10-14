use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, Fields, Ident, LitStr, Path, Token, Type};

struct Variant {
    ident: Ident,
    shape: Shape,
    guards: Vec<Path>,
    at: Option<Pattern>,
}

enum Shape {
    Unit,
    Branch(Type),
    Leaf(Vec<(Ident, Type)>),
}

struct Pattern {
    span: Span,
    segments: Vec<Segment>,
}

enum Segment {
    Literal(String),
    Capture(Ident),
}

impl Pattern {
    fn parse(literal: &LitStr) -> syn::Result<Self> {
        let text = literal.value();
        let span = literal.span();

        let Some(path) = text.strip_prefix('/') else {
            return Err(syn::Error::new(span, "a link starts with `/`"));
        };

        let mut segments = Vec::new();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            match segment.strip_prefix(':') {
                Some(name) => match syn::parse_str::<Ident>(name) {
                    Ok(field) => segments.push(Segment::Capture(field)),
                    Err(_) => {
                        return Err(syn::Error::new(span, format!("`:{name}` does not name a field")));
                    }
                },
                None if segment.contains(':') => {
                    return Err(syn::Error::new(span, format!("`{segment}`: a capture is a whole segment, `:field`")));
                }
                None => segments.push(Segment::Literal(segment.to_owned())),
            }
        }

        Ok(Self { span, segments })
    }

    fn shape(&self) -> Vec<Option<&str>> {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Literal(text) => Some(text.as_str()),
                Segment::Capture(_) => None,
            })
            .collect()
    }

    fn order(&self) -> Vec<bool> {
        self.segments.iter().map(|segment| matches!(segment, Segment::Capture(_))).collect()
    }

    fn text(&self) -> String {
        let written: Vec<String> = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Literal(text) => text.clone(),
                Segment::Capture(field) => format!(":{field}"),
            })
            .collect();
        format!("/{}", written.join("/"))
    }
}

pub fn derive(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let navigator = crate::paths::router("navigator");
    let enter = crate::paths::router("enter");
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(name.span(), "a route is an enum, a variant per place"));
    };

    let outer = guards_of(&input.attrs)?;
    if at_of(&input.attrs)?.is_some() {
        return Err(syn::Error::new(name.span(), "`#[at]` goes on a variant, not on the enum"));
    }

    let variants = data.variants.iter().map(variant).collect::<syn::Result<Vec<_>>>()?;
    refuse_same_links(&variants)?;

    let names = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let text = ident.to_string();
        quote!(Self::#ident { .. } => #text)
    });

    let branches: Vec<_> = variants
        .iter()
        .filter_map(|variant| match &variant.shape {
            Shape::Branch(nested) => Some((&variant.ident, nested)),
            _ => None,
        })
        .collect();

    let stays = branches.iter().map(|(ident, _)| quote!((Self::#ident(_), Self::#ident(_)) => true,));

    let stand = |guard: &Path| quote!(&#enter::Stands::<#guard>(::core::marker::PhantomData) as &'static dyn #enter::EnterGuard);
    let outer_guards = outer.iter().map(stand);
    let variant_guards = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let own = variant.guards.iter().map(stand);
        let nested = match &variant.shape {
            Shape::Branch(_) => quote!(guards.extend(nested.guards());),
            _ => quote!(),
        };
        let binding = match &variant.shape {
            Shape::Branch(_) => quote!(Self::#ident(nested)),
            _ => quote!(Self::#ident { .. }),
        };
        quote!(#binding => { #(guards.push(#own);)* #nested })
    });

    let levels = branches.iter().map(|(ident, nested)| {
        quote! {
            levels.extend(#navigator::Level::<Self>::nested::<#nested>(
                |route| match route { Self::#ident(nested) => ::core::option::Option::Some(nested), _ => ::core::option::Option::None },
                Self::#ident,
            ));
        }
    });

    let sample = crate::paths::router("sample");
    let samples = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let text = ident.to_string();
        match &variant.shape {
            Shape::Unit => quote!(samples.push(::core::result::Result::Ok(Self::#ident));),
            Shape::Branch(nested) => quote! {
                samples.extend(
                    <#nested as #navigator::Route>::samples()
                        .into_iter()
                        .map(|nested| nested.map(Self::#ident)),
                );
            },
            Shape::Leaf(fields) => {
                let made = fields.iter().map(|(name, ty)| {
                    quote!(#name: (&#sample::Probe::<#ty>::new()).make()?)
                });
                quote! {
                    samples.push(
                        (|| -> ::core::option::Option<Self> {
                            use #sample::{ByDefault as _, Without as _};
                            ::core::option::Option::Some(Self::#ident { #(#made),* })
                        })()
                        .ok_or(#text),
                    );
                }
            }
        }
    });

    let link = crate::paths::router("link");
    let writes = variants.iter().map(|variant| write(&link, variant));

    let tree = name.to_string();
    let named = |guard: &Path| quote!(#enter::EnterGuard::name(&#enter::Stands::<#guard>(::core::marker::PhantomData)));
    let outer_names: Vec<TokenStream> = outer.iter().map(named).collect();
    let deep_links = variants.iter().filter_map(|variant| {
        let pattern = variant.at.as_ref()?;
        let ident = &variant.ident;
        let route = ident.to_string();
        let path = pattern.text();
        let guards = outer_names.iter().cloned().chain(variant.guards.iter().map(named));

        Some(match &variant.shape {
            Shape::Branch(nested) => quote! {
                for nested in <#nested as #navigator::Route>::deep_links() {
                    let mut guards: ::std::vec::Vec<&'static str> = ::std::vec![#(#guards),*];
                    guards.extend(nested.guards);
                    let path = match (#path, nested.path.as_str()) {
                        ("/", path) => ::std::string::String::from(path),
                        (prefix, "/") => ::std::string::String::from(prefix),
                        (prefix, path) => ::std::format!("{prefix}{path}"),
                    };
                    links.push(#link::DeepLink {
                        tree: #tree,
                        route: ::std::format!("{}::{}", #route, nested.route),
                        path,
                        captures: nested.captures,
                        guards,
                    });
                }
            },
            Shape::Unit | Shape::Leaf(_) => {
                let fields: &[(Ident, Type)] = match &variant.shape {
                    Shape::Leaf(fields) => fields,
                    _ => &[],
                };
                let captures = pattern.segments.iter().filter_map(|segment| {
                    let Segment::Capture(field) = segment else { return None };
                    let ty = &fields.iter().find(|(name, _)| name == field)?.1;
                    let text = field.to_string();
                    Some(quote!(#link::Capture { name: #text, ty: <#ty as #link::LinkValue>::NAME }))
                });
                quote! {
                    links.push(#link::DeepLink {
                        tree: #tree,
                        route: ::std::string::String::from(#route),
                        path: ::std::string::String::from(#path),
                        captures: ::std::vec![#(#captures),*],
                        guards: ::std::vec![#(#guards),*],
                    });
                }
            }
        })
    });

    let mut ordered: Vec<&Variant> = variants.iter().filter(|variant| variant.at.is_some()).collect();
    ordered.sort_by_key(|variant| matches!(variant.shape, Shape::Branch(_)));
    let reads = ordered.into_iter().map(|variant| read(&navigator, &link, variant));

    Ok(quote! {
        impl #impl_generics #navigator::Route for #name #ty_generics #where_clause {
            fn name(&self) -> &'static str {
                match self {
                    #(#names,)*
                }
            }

            fn stays(&self, to: &Self) -> bool {
                #[allow(unreachable_patterns)]
                match (self, to) {
                    #(#stays)*
                    _ => self == to,
                }
            }

            fn guards(&self) -> ::std::vec::Vec<&'static dyn #enter::EnterGuard> {
                let mut guards: ::std::vec::Vec<&'static dyn #enter::EnterGuard> = ::std::vec![#(#outer_guards),*];

                match self {
                    #(#variant_guards)*
                }

                guards
            }

            fn levels() -> ::std::vec::Vec<#navigator::Level<Self>> {
                #[allow(unused_mut)]
                let mut levels = ::std::vec::Vec::new();
                #(#levels)*
                levels
            }

            fn samples() -> ::std::vec::Vec<::core::result::Result<Self, &'static str>> {
                let mut samples = ::std::vec::Vec::new();
                #(#samples)*
                samples
            }

            fn deep_links() -> ::std::vec::Vec<#link::DeepLink> {
                #[allow(unused_mut)]
                let mut links = ::std::vec::Vec::new();
                #(#deep_links)*
                links
            }

            fn write(&self, into: &mut ::std::vec::Vec<::std::string::String>) -> bool {
                #[allow(unused_variables)]
                match self {
                    #(#writes)*
                }
            }

            fn read(segments: &[&str]) -> ::core::option::Option<(Self, ::std::vec::Vec<bool>)> {
                #[allow(unused_mut)]
                let mut best: ::core::option::Option<(Self, ::std::vec::Vec<bool>)> = ::core::option::Option::None;
                #(#reads)*
                best
            }
        }
    })
}

fn variant(variant: &syn::Variant) -> syn::Result<Variant> {
    let ident = variant.ident.clone();

    let shape = match &variant.fields {
        Fields::Unit => Shape::Unit,
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => Shape::Branch(fields.unnamed[0].ty.clone()),
        Fields::Unnamed(fields) => {
            return Err(syn::Error::new(
                fields.span(),
                "a tuple variant holds the route of the level below, one field; a place with parameters names them: `Process { pid: u32 }`",
            ));
        }
        Fields::Named(fields) => Shape::Leaf(
            fields
                .named
                .iter()
                .map(|field| (field.ident.clone().expect("a named field"), field.ty.clone()))
                .collect(),
        ),
    };

    let guards = guards_of(&variant.attrs)?;
    let at = at_of(&variant.attrs)?;

    if let Some(pattern) = &at {
        check_captures(&ident, &shape, pattern)?;
    }

    Ok(Variant { ident, shape, guards, at })
}

fn check_captures(ident: &Ident, shape: &Shape, pattern: &Pattern) -> syn::Result<()> {
    let captured: Vec<&Ident> = pattern
        .segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Capture(field) => Some(field),
            Segment::Literal(_) => None,
        })
        .collect();

    let fields: &[(Ident, Type)] = match shape {
        Shape::Leaf(fields) => fields,
        Shape::Unit | Shape::Branch(_) => &[],
    };

    for field in &captured {
        if !fields.iter().any(|(name, _)| name == *field) {
            return Err(syn::Error::new(pattern.span, format!("`:{field}`: `{ident}` has no field `{field}`")));
        }
        if captured.iter().filter(|other| other == &field).count() > 1 {
            return Err(syn::Error::new(pattern.span, format!("`:{field}` is captured twice")));
        }
    }

    for (name, _) in fields {
        if !captured.contains(&name) {
            return Err(syn::Error::new(
                pattern.span,
                format!("`{ident}.{name}` is not in its link, so the link cannot be read back: add `:{name}`"),
            ));
        }
    }

    Ok(())
}

fn refuse_same_links(variants: &[Variant]) -> syn::Result<()> {
    for (index, variant) in variants.iter().enumerate() {
        let Some(pattern) = &variant.at else { continue };
        let branch = matches!(variant.shape, Shape::Branch(_));

        for earlier in &variants[..index] {
            let Some(other) = &earlier.at else { continue };
            if branch || matches!(earlier.shape, Shape::Branch(_)) {
                continue;
            }
            if pattern.shape() == other.shape() {
                return Err(syn::Error::new(
                    pattern.span,
                    format!("`{}` answers to the same links as `{}`", variant.ident, earlier.ident),
                ));
            }
        }
    }

    Ok(())
}

fn write(link: &TokenStream, variant: &Variant) -> TokenStream {
    let ident = &variant.ident;

    let binding = match &variant.shape {
        Shape::Unit => quote!(Self::#ident),
        Shape::Branch(_) => quote!(Self::#ident(nested)),
        Shape::Leaf(fields) => {
            let names = fields.iter().map(|(name, _)| name);
            quote!(Self::#ident { #(#names),* })
        }
    };

    let Some(pattern) = &variant.at else {
        return quote!(#binding => false,);
    };

    let pushes = pattern.segments.iter().map(|segment| match segment {
        Segment::Literal(text) => quote!(into.push(::std::string::String::from(#text));),
        Segment::Capture(field) => {
            let ty = match &variant.shape {
                Shape::Leaf(fields) => fields.iter().find(|(name, _)| name == field).map(|(_, ty)| ty),
                _ => None,
            };
            quote!(into.push(<#ty as #link::LinkValue>::encode(#field));)
        }
    });

    let rest = match &variant.shape {
        Shape::Branch(_) => quote!(nested.write(into)),
        _ => quote!(true),
    };

    quote!(#binding => { #(#pushes)* #rest })
}

fn read(navigator: &TokenStream, link: &TokenStream, variant: &Variant) -> TokenStream {
    let ident = &variant.ident;
    let pattern = variant.at.as_ref().expect("only linked variants are read");
    let count = pattern.segments.len();

    let matches = pattern.segments.iter().enumerate().map(|(index, segment)| match segment {
        Segment::Literal(text) => quote!(if segments[#index] != #text { return ::core::option::Option::None; }),
        Segment::Capture(field) => {
            let local = format_ident!("__{}", field);
            quote!(let #local = segments[#index];)
        }
    });

    let order = pattern.order();

    let (length, built) = match &variant.shape {
        Shape::Unit => (
            quote!(segments.len() == #count),
            quote!((Self::#ident, ::std::vec![#(#order),*])),
        ),
        Shape::Branch(nested) => (
            quote!(segments.len() >= #count),
            quote! {{
                let (nested, below) = <#nested as #navigator::Route>::read(&segments[#count..])?;
                let mut order = ::std::vec![#(#order),*];
                order.extend(below);
                (Self::#ident(nested), order)
            }},
        ),
        Shape::Leaf(fields) => {
            let parsed = fields.iter().map(|(name, ty)| {
                let local = format_ident!("__{}", name);
                quote!(#name: <#ty as #link::LinkValue>::decode(#local)?)
            });
            (
                quote!(segments.len() == #count),
                quote!((Self::#ident { #(#parsed),* }, ::std::vec![#(#order),*])),
            )
        }
    };

    quote! {
        if #length {
            let attempt = (|| -> ::core::option::Option<(Self, ::std::vec::Vec<bool>)> {
                #(#matches)*
                ::core::option::Option::Some(#built)
            })();
            if let ::core::option::Option::Some(found) = attempt
                && best.as_ref().is_none_or(|(_, order)| found.1 < *order)
            {
                best = ::core::option::Option::Some(found);
            }
        }
    }
}

fn guards_of(attrs: &[syn::Attribute]) -> syn::Result<Vec<Path>> {
    let mut guards = Vec::new();

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("guard")) {
        let listed = attr.parse_args_with(syn::punctuated::Punctuated::<Path, Token![,]>::parse_terminated)?;
        if listed.is_empty() {
            return Err(syn::Error::new(attr.span(), "`#[guard(..)]` names the guard types: `#[guard(Signed)]`"));
        }
        guards.extend(listed);
    }

    Ok(guards)
}

fn at_of(attrs: &[syn::Attribute]) -> syn::Result<Option<Pattern>> {
    let mut found = None;

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("at")) {
        if found.is_some() {
            return Err(syn::Error::new(attr.span(), "a variant has one `#[at]`"));
        }
        let literal: LitStr = attr.parse_args()?;
        found = Some(Pattern::parse(&literal)?);
    }

    Ok(found)
}
