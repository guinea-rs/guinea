use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Delimiter, Ident, Span, TokenStream, TokenTree};
use quote::{format_ident, quote, quote_spanned};
use winnow::error::{ContextError, ErrMode};
use winnow::prelude::*;
use winnow::token::any;

use crate::handler::guinea_core_crate_path;

type Tokens<'i> = &'i [TokenTree];

struct Manifest {
    ident: Ident,
    generics: syn::Generics,
    self_ty: syn::Type,
    handlers: Vec<HandlerDecl>,
    publishes: Vec<syn::Type>,
    subscribes: Vec<syn::Type>,
}

struct HandlerDecl {
    msg: syn::Type,
    /// `None` when the handler declares no outgoing messages at all, which is
    /// different from declaring an empty set.
    edges: Option<Vec<Edge>>,
}

struct Edge {
    channel: Channel,
    target: syn::Type,
    is_loop: bool,
    span: Span,
}

#[derive(PartialEq)]
enum Channel {
    Send,
    Bg,
    Emit,
    Ask,
}

pub fn actor_impl(input: TokenStream1) -> TokenStream1 {
    let tokens: Vec<TokenTree> = TokenStream::from(input).into_iter().collect();
    match parse_manifest(&tokens).and_then(expand) {
        Ok(ts) => ts.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn fail<O>() -> ModalResult<O> {
    Err(ErrMode::Backtrack(ContextError::new()))
}

fn any_ident<'i>(input: &mut Tokens<'i>) -> ModalResult<Ident> {
    match any.parse_next(input)? {
        TokenTree::Ident(id) => Ok(id),
        _ => fail(),
    }
}

fn braced<'i>(input: &mut Tokens<'i>) -> ModalResult<Vec<TokenTree>> {
    match any.parse_next(input)? {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            Ok(g.stream().into_iter().collect())
        }
        _ => fail(),
    }
}

fn parse_manifest(tokens: &[TokenTree]) -> syn::Result<Manifest> {
    let brace_at = tokens
        .iter()
        .position(|tt| matches!(tt, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace))
        .ok_or_else(|| {
            syn::Error::new(
                span_of(tokens),
                "expected `ActorType<..> { .. }` - the body brace is missing",
            )
        })?;

    let (header, rest) = tokens.split_at(brace_at);
    let (ident, generics) = parse_header(header)?;
    let (_, ty_generics, _) = generics.split_for_impl();
    let self_ty: syn::Type = syn::parse2(quote!(#ident #ty_generics))?;

    let mut manifest = Manifest {
        ident,
        generics,
        self_ty,
        handlers: Vec::new(),
        publishes: Vec::new(),
        subscribes: Vec::new(),
    };

    let TokenTree::Group(body) = &rest[0] else {
        unreachable!("position matched a brace group")
    };
    parse_sections(&body.stream().into_iter().collect::<Vec<_>>(), &mut manifest)?;

    if manifest.handlers.is_empty() {
        return Err(syn::Error::new(
            manifest.ident.span(),
            "an actor must declare at least one handler: `handlers { .. }`",
        ));
    }

    Ok(manifest)
}

/// The header reads like a struct declaration, so it is parsed as one: that
/// gives bounds and `where` clauses for free.
fn parse_header(header: &[TokenTree]) -> syn::Result<(Ident, syn::Generics)> {
    if header.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "expected an actor type before the body",
        ));
    }
    let ts: TokenStream = header.iter().cloned().collect();
    let item: syn::ItemStruct = syn::parse2(quote!(struct #ts {}))
        .map_err(|e| syn::Error::new(span_of(header), format!("invalid actor type: {e}")))?;
    Ok((item.ident, item.generics))
}

fn parse_sections(tokens: &[TokenTree], manifest: &mut Manifest) -> syn::Result<()> {
    let mut slice: Tokens = tokens;

    while !slice.is_empty() {
        let name = any_ident
            .parse_next(&mut slice)
            .map_err(|_| syn::Error::new(span_of(slice), "expected a section name"))?;

        let body = braced
            .parse_next(&mut slice)
            .map_err(|_| syn::Error::new(name.span(), "expected `{ .. }` after the section name"))?;

        match name.to_string().as_str() {
            "handlers" => manifest.handlers = parse_handlers(&body)?,
            "publishes" => manifest.publishes = parse_type_list(&body)?,
            "subscribes" => manifest.subscribes = parse_type_list(&body)?,
            other => {
                return Err(syn::Error::new(
                    name.span(),
                    format!("unknown section `{other}`; expected handlers, publishes or subscribes"),
                ));
            }
        }
    }

    Ok(())
}

/// `Msg` or `Msg => { send Other, bg Third }`, comma-separated; the comma
/// after a block is optional, as with match arms.
fn parse_handlers(tokens: &[TokenTree]) -> syn::Result<Vec<HandlerDecl>> {
    let mut decls = Vec::new();
    let mut pending: Vec<TokenTree> = Vec::new();
    let mut iter = tokens.iter().peekable();

    while let Some(tt) = iter.next() {
        match tt {
            TokenTree::Punct(p) if p.as_char() == ',' => {
                if !pending.is_empty() {
                    decls.push(HandlerDecl {
                        msg: parse_one_type(&mut pending)?,
                        edges: None,
                    });
                }
            }
            TokenTree::Punct(p) if p.as_char() == '=' => {
                let arrow = matches!(iter.peek(), Some(TokenTree::Punct(g)) if g.as_char() == '>');
                if !arrow {
                    pending.push(tt.clone());
                    continue;
                }
                iter.next();

                let Some(TokenTree::Group(body)) = iter.next() else {
                    return Err(syn::Error::new(p.span(), "expected `{ .. }` after `=>`"));
                };
                if body.delimiter() != Delimiter::Brace {
                    return Err(syn::Error::new(body.span(), "expected `{ .. }` after `=>`"));
                }

                let msg = parse_one_type(&mut pending)?;
                let edges = parse_edges(&body.stream().into_iter().collect::<Vec<_>>())?;
                decls.push(HandlerDecl {
                    msg,
                    edges: Some(edges),
                });

                if matches!(iter.peek(), Some(TokenTree::Punct(p)) if p.as_char() == ',') {
                    iter.next();
                }
            }
            other => pending.push(other.clone()),
        }
    }

    if !pending.is_empty() {
        decls.push(HandlerDecl {
            msg: parse_one_type(&mut pending)?,
            edges: None,
        });
    }

    Ok(decls)
}

fn parse_edges(tokens: &[TokenTree]) -> syn::Result<Vec<Edge>> {
    let mut edges = Vec::new();

    for chunk in split_on_commas(tokens) {
        let mut slice: Tokens = &chunk;
        let Some(TokenTree::Ident(head)) = slice.first().cloned() else {
            return Err(syn::Error::new(
                span_of(&chunk),
                "expected `send`, `bg`, `emit` or `ask` before the message type",
            ));
        };

        let channel = match head.to_string().as_str() {
            "send" => Channel::Send,
            "bg" => Channel::Bg,
            "emit" => Channel::Emit,
            "ask" => Channel::Ask,
            other => {
                return Err(syn::Error::new(
                    head.span(),
                    format!("unknown channel `{other}`; expected send, bg, emit or ask"),
                ));
            }
        };
        slice = &slice[1..];

        if channel == Channel::Emit {
            if let Some(TokenTree::Ident(scope)) = slice.first() {
                let scope = scope.to_string();
                if scope == "local" || scope == "global" {
                    slice = &slice[1..];
                }
            }
        }

        let mut rest: Vec<TokenTree> = slice.to_vec();
        let mut is_loop = false;
        if let Some(TokenTree::Ident(last)) = rest.last() {
            if last == "loop" {
                is_loop = true;
                rest.pop();
            }
        }

        edges.push(Edge {
            channel,
            target: parse_one_type(&mut rest)?,
            is_loop,
            span: head.span(),
        });
    }

    Ok(edges)
}

fn split_on_commas(tokens: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for tt in tokens {
        if matches!(tt, TokenTree::Punct(p) if p.as_char() == ',') {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(tt.clone());
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_one_type(tokens: &mut Vec<TokenTree>) -> syn::Result<syn::Type> {
    let span = span_of(tokens);
    let ts: TokenStream = tokens.drain(..).collect();
    syn::parse2(ts).map_err(|e| syn::Error::new(span, format!("expected a type: {e}")))
}

fn parse_type_list(tokens: &[TokenTree]) -> syn::Result<Vec<syn::Type>> {
    let mut types = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();

    for tt in tokens {
        if matches!(tt, TokenTree::Punct(p) if p.as_char() == ',') {
            push_type(&mut current, &mut types)?;
        } else {
            current.push(tt.clone());
        }
    }
    push_type(&mut current, &mut types)?;

    Ok(types)
}

fn push_type(current: &mut Vec<TokenTree>, out: &mut Vec<syn::Type>) -> syn::Result<()> {
    if current.is_empty() {
        return Ok(());
    }
    let span = span_of(current);
    let ts: TokenStream = current.drain(..).collect();
    out.push(syn::parse2(ts).map_err(|e| syn::Error::new(span, format!("expected a type: {e}")))?);
    Ok(())
}

fn span_of(tokens: &[TokenTree]) -> Span {
    tokens
        .first()
        .map(|tt| tt.span())
        .unwrap_or_else(Span::call_site)
}

/// A cycle among an actor's own messages is legal - self-restarting timers are
/// the usual case - but only when said out loud with `loop` on one of its
/// edges.
fn check_cycles(handlers: &[HandlerDecl]) -> syn::Result<()> {
    let key = |ty: &syn::Type| quote!(#ty).to_string();

    let mut graph: std::collections::HashMap<String, Vec<&Edge>> =
        std::collections::HashMap::new();
    for decl in handlers {
        let edges = decl.edges.iter().flatten().collect::<Vec<_>>();
        graph.insert(key(&decl.msg), edges);
    }

    let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();

    for decl in handlers {
        let start = key(&decl.msg);
        if done.contains(&start) {
            continue;
        }

        let mut stack = vec![(start.clone(), Vec::<&Edge>::new())];
        let mut on_path: std::collections::HashSet<String> = std::collections::HashSet::new();

        while let Some((node, path)) = stack.pop() {
            if !on_path.insert(node.clone()) {
                continue;
            }
            done.insert(node.clone());

            for edge in graph.get(&node).into_iter().flatten() {
                let target = key(&edge.target);
                let mut next = path.clone();
                next.push(edge);

                if target == start {
                    if !next.iter().any(|e| e.is_loop) {
                        return Err(syn::Error::new(
                            edge.span,
                            format!(
                                "`{}` can send its way back to itself; mark one edge of the cycle with `loop` if that is intended",
                                start
                            ),
                        ));
                    }
                    continue;
                }

                if graph.contains_key(&target) {
                    stack.push((target, next));
                }
            }
        }
    }

    Ok(())
}

fn expand(manifest: Manifest) -> syn::Result<TokenStream> {
    check_cycles(&manifest.handlers)?;

    let gc = guinea_core_crate_path();
    let Manifest {
        ident,
        generics,
        self_ty,
        handlers,
        publishes,
        subscribes,
    } = manifest;

    let (impl_generics, _, where_clause) = generics.split_for_impl();

    let signals_marker = format_ident!("__Signals_{}", ident);
    let bus_marker = format_ident!("__Bus_{}", ident);

    let handler_asserts = handlers.iter().map(|decl| {
        let msg = &decl.msg;
        quote! { assert_handler::<#self_ty, #msg>(); }
    });

    // Registering what this actor answers, from the very list that declares
    // it. Nothing outside names the actor: the scope is keyed by the action,
    // so an actor is what the domain happens to use, not part of anything's
    // signature.
    let served = handlers.iter().map(|decl| {
        let msg = &decl.msg;
        quote! {
            scope.answers::<#msg>({
                let addr = addr.clone();
                move |action| addr.send(action)
            });
        }
    });

    let flow_marker = format_ident!("__Flow_{}", ident);
    let declares_flow = handlers.iter().any(|decl| decl.edges.is_some());

    let mut seen = std::collections::HashSet::new();
    let allow_impls: Vec<TokenStream> = handlers
        .iter()
        .filter_map(|decl| decl.edges.as_ref().map(|edges| (&decl.msg, edges)))
        .flat_map(|(msg, edges)| edges.iter().map(move |edge| (msg, edge)))
        .filter(|(_, edge)| matches!(edge.channel, Channel::Send | Channel::Bg))
        .filter_map(|(msg, edge)| {
            let target = &edge.target;
            let key = quote!(#msg => #target).to_string();
            if !seen.insert(key) {
                return None;
            }
            Some(quote! { impl #gc::actor::flow::Allows<#msg, #target> for #flow_marker {} })
        })
        .collect();

    let (flow_ty, flow_decl) = if declares_flow {
        (
            quote!(#flow_marker),
            quote! {
                #[doc(hidden)]
                #[allow(non_camel_case_types)]
                pub struct #flow_marker;
                #(#allow_impls)*
            },
        )
    } else {
        (quote!(#gc::actor::flow::Open), quote!())
    };

    let signal_impls = publishes.iter().map(|msg| {
        quote! { impl #gc::actor::traits::AllowedSignal<#msg> for #signals_marker {} }
    });

    let subscriptions = subscribes.iter().map(|msg| {
        quote! {
            <#msg as #gc::actor::event_bus::builder::EventSubscription<#self_ty>>::subscribe_into(
                addr.clone(),
                tracker,
            );
        }
    });

    let channel_of = |channel: &Channel| match channel {
        Channel::Send => quote!(#gc::actor::shape::Channel::Send),
        Channel::Bg => quote!(#gc::actor::shape::Channel::Bg),
        Channel::Emit => quote!(#gc::actor::shape::Channel::Emit),
        Channel::Ask => quote!(#gc::actor::shape::Channel::Ask),
    };
    let shape_handles = handlers.iter().map(|decl| {
        let msg = &decl.msg;
        let edges = match &decl.edges {
            None => quote!(::core::option::Option::None),
            Some(edges) => {
                let edges = edges.iter().map(|edge| {
                    let channel = channel_of(&edge.channel);
                    let target = &edge.target;
                    let looping = edge.is_loop;
                    quote! {
                        #gc::actor::shape::Edge {
                            channel: #channel,
                            target: #gc::actor::shape::name::<#target>,
                            looping: #looping,
                        }
                    }
                });
                quote!(::core::option::Option::Some(&[#(#edges),*]))
            }
        };
        quote! {
            #gc::actor::shape::Handles {
                message: #gc::actor::shape::name::<#msg>,
                edges: #edges,
                declared: <#self_ty as #gc::actor::Handler<#msg>>::DECLARED,
            }
        }
    });
    let shape_publishes = publishes
        .iter()
        .map(|msg| quote!(#gc::actor::shape::name::<#msg>));
    let shape_subscribes = subscribes
        .iter()
        .map(|msg| quote!(#gc::actor::shape::name::<#msg>));

    let declared = quote_spanned! {ident.span()=>
        #gc::actor::shape::Declared {
            file: ::core::file!(),
            line: ::core::line!(),
            column: ::core::column!(),
            crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
        }
    };

    let bus_ty = if subscribes.is_empty() {
        quote!(())
    } else {
        quote!(#bus_marker)
    };

    let bus_impl = if subscribes.is_empty() {
        quote!()
    } else {
        quote! {
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub struct #bus_marker;

            impl #impl_generics #gc::actor::event_bus::builder::EventSubscription<#self_ty>
                for #bus_marker #where_clause
            {
                fn subscribe_into(
                    addr: #gc::actor::Addr<#self_ty>,
                    tracker: &impl #gc::lifecycle_tracker::LifecycleTracker,
                ) {
                    #(#subscriptions)*
                }
            }
        }
    };

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #signals_marker;

        #bus_impl

        #flow_decl

        #(#signal_impls)*

        impl #impl_generics #gc::feature::Serves for #self_ty #where_clause {
            fn serve(
                addr: &#gc::actor::Addr<Self>,
                scope: #gc::scope::Scope,
            ) {
                #(#served)*
            }
        }

        impl #impl_generics #gc::actor::traits::ManagedActor for #self_ty #where_clause {
            type Bus = #bus_ty;
            type Signals = #signals_marker;
            type Flow = #flow_ty;
            const SHAPE: #gc::actor::shape::Shape = #gc::actor::shape::Shape {
                handles: &[#(#shape_handles),*],
                publishes: &[#(#shape_publishes),*],
                subscribes: &[#(#shape_subscribes),*],
                declared: ::core::option::Option::Some(#declared),
            };
        }

        const _: () = {
            fn check_handlers #impl_generics () #where_clause {
                fn assert_handler<A, M>()
                where
                    A: #gc::actor::traits::Handler<M>,
                    M: 'static,
                {
                }
                #(#handler_asserts)*
            }
        };
    })
}
