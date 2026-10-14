use std::collections::{HashMap, HashSet};

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Delimiter, Ident, Spacing, Span, TokenStream, TokenTree};
use quote::{format_ident, quote, quote_spanned};
use winnow::error::{ContextError, ErrMode};
use winnow::prelude::*;
use winnow::token::any;


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
    Task,
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

/// `Msg` or `Msg => { send Other, task Third }`, comma-separated; the comma
/// after a block is optional, as with match arms.
fn parse_handlers(tokens: &[TokenTree]) -> syn::Result<Vec<HandlerDecl>> {
    let mut decls = Vec::new();
    let mut pending: Vec<TokenTree> = Vec::new();
    let mut depth = 0;
    let mut iter = tokens.iter().peekable();

    while let Some(tt) = iter.next() {
        match tt {
            TokenTree::Punct(p) if depth == 0 && p.as_char() == ',' => {
                if !pending.is_empty() {
                    decls.push(HandlerDecl {
                        msg: parse_one_type(&mut pending)?,
                        edges: None,
                    });
                }
            }
            TokenTree::Punct(p) if depth == 0 && p.as_char() == '=' => {
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
            other => {
                depth += angle_step(pending.last(), other);
                pending.push(other.clone());
            }
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
                "expected `send`, `task`, `emit` or `ask` before the message type",
            ));
        };

        let channel = match head.to_string().as_str() {
            "send" => Channel::Send,
            "task" => Channel::Task,
            "emit" => Channel::Emit,
            "ask" => Channel::Ask,
            other => {
                return Err(syn::Error::new(
                    head.span(),
                    format!("unknown channel `{other}`; expected send, task, emit or ask"),
                ));
            }
        };
        slice = &slice[1..];

        if channel == Channel::Emit {
            if let Some(TokenTree::Ident(scope)) = slice.first() {
                let scope = scope.to_string();
                let a_path = matches!(slice.get(1), Some(TokenTree::Punct(p)) if p.as_char() == ':');
                if (scope == "local" || scope == "global") && !a_path {
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

/// `tokens` cut at each comma outside `<..>`, without the empty pieces.
fn split_on_commas(tokens: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut out = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();
    let mut depth = 0;

    for tt in tokens {
        if depth == 0 && matches!(tt, TokenTree::Punct(p) if p.as_char() == ',') {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        depth += angle_step(current.last(), tt);
        current.push(tt.clone());
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// How far `tt` takes the tokens into `<..>` or out of it, after `previous`:
/// the `>` of `->` or `=>` closes nothing.
fn angle_step(previous: Option<&TokenTree>, tt: &TokenTree) -> isize {
    let TokenTree::Punct(punct) = tt else {
        return 0;
    };

    match punct.as_char() {
        '<' => 1,
        '>' => match previous {
            Some(TokenTree::Punct(before))
                if before.spacing() == Spacing::Joint && matches!(before.as_char(), '-' | '=') =>
            {
                0
            }
            _ => -1,
        },
        _ => 0,
    }
}

fn parse_one_type(tokens: &mut Vec<TokenTree>) -> syn::Result<syn::Type> {
    let span = span_of(tokens);
    let ts: TokenStream = tokens.drain(..).collect();
    syn::parse2(ts).map_err(|e| syn::Error::new(span, format!("expected a type: {e}")))
}

fn parse_type_list(tokens: &[TokenTree]) -> syn::Result<Vec<syn::Type>> {
    split_on_commas(tokens)
        .into_iter()
        .map(|mut chunk| parse_one_type(&mut chunk))
        .collect()
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
///
/// Every cycle has a `loop` edge exactly when the edges without one form no
/// cycle, so that is what is searched.
fn check_cycles(handlers: &[HandlerDecl]) -> syn::Result<()> {
    let key = |ty: &syn::Type| quote!(#ty).to_string();

    let mut graph: HashMap<String, Vec<(String, &Edge)>> = HashMap::new();
    for decl in handlers {
        let edges = decl
            .edges
            .iter()
            .flatten()
            .filter(|edge| !edge.is_loop)
            .map(|edge| (key(&edge.target), edge))
            .collect();
        graph.insert(key(&decl.msg), edges);
    }

    let mut finished = HashSet::new();
    let mut path = Vec::new();
    for decl in handlers {
        walk(&key(&decl.msg), &graph, &mut path, &mut finished)?;
    }

    Ok(())
}

/// Depth first from `node`: `path` is what leads to it, `finished` what was
/// already searched to the end.
fn walk(
    node: &str,
    graph: &HashMap<String, Vec<(String, &Edge)>>,
    path: &mut Vec<String>,
    finished: &mut HashSet<String>,
) -> syn::Result<()> {
    if finished.contains(node) {
        return Ok(());
    }
    path.push(node.to_owned());

    for (target, edge) in graph.get(node).into_iter().flatten() {
        if let Some(at) = path.iter().position(|on_path| on_path == target) {
            let cycle = path[at..]
                .iter()
                .chain([target])
                .map(|message| format!("`{message}`"))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(syn::Error::new(
                edge.span,
                format!("{cycle} is a cycle; mark one edge of it with `loop` if that is intended"),
            ));
        }

        if graph.contains_key(target) {
            walk(target, graph, path, finished)?;
        }
    }

    path.pop();
    finished.insert(node.to_owned());
    Ok(())
}

/// The `Req` of `RpcRequest<Req>`. The handler that answers it is handed
/// `Cx<_, Req>`, so it sends as `Req` and the request's edges are `Req`'s too.
fn request_payload(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "RpcRequest" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };

    match arguments.args.first() {
        Some(syn::GenericArgument::Type(payload)) if arguments.args.len() == 1 => Some(payload),
        _ => None,
    }
}

/// A request handled beside its own payload as a message: both handlers send
/// as the payload, so the edges either declares would be the other's too.
fn check_requests(handlers: &[HandlerDecl]) -> syn::Result<()> {
    let key = |ty: &syn::Type| quote!(#ty).to_string();

    for request in handlers {
        let Some(payload) = request_payload(&request.msg) else {
            continue;
        };
        let payload = key(payload);
        let Some(message) = handlers.iter().find(|decl| key(&decl.msg) == payload) else {
            continue;
        };

        if request.edges.is_some() || message.edges.is_some() {
            let request = key(&request.msg);
            return Err(syn::Error::new_spanned(
                &message.msg,
                format!(
                    "`{payload}` is handled here both as a message and as `{request}`, and both handlers send as `{payload}`: the edges one declares would be the other's too; handle one of them in another actor"
                ),
            ));
        }
    }

    Ok(())
}

fn expand(manifest: Manifest) -> syn::Result<TokenStream> {
    check_cycles(&manifest.handlers)?;
    check_requests(&manifest.handlers)?;

    let gc = crate::paths::core();
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
            section.answers::<#msg>({
                let addr = addr.clone();
                move |action| addr.send(action)
            });
        }
    });

    let flow_marker = format_ident!("__Flow_{}", ident);
    let declares_flow = handlers.iter().any(|decl| decl.edges.is_some());

    let mut seen = HashSet::new();
    let allow_impls: Vec<TokenStream> = handlers
        .iter()
        .filter_map(|decl| decl.edges.as_ref().map(|edges| (&decl.msg, edges)))
        .flat_map(|(msg, edges)| {
            std::iter::once(msg)
                .chain(request_payload(msg))
                .flat_map(move |sender| edges.iter().map(move |edge| (sender, edge)))
        })
        .filter(|(_, edge)| matches!(edge.channel, Channel::Send | Channel::Task))
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
            <#msg as #gc::actor::traits::EventSubscription<#self_ty>>::subscribe_into(addr);
        }
    });

    let channel_of = |channel: &Channel| match channel {
        Channel::Send => quote!(#gc::actor::shape::Channel::Send),
        Channel::Task => quote!(#gc::actor::shape::Channel::Task),
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

            impl #impl_generics #gc::actor::traits::EventSubscription<#self_ty>
                for #bus_marker #where_clause
            {
                fn subscribe_into(addr: &#gc::actor::Addr<#self_ty>) {
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
                section: #gc::scope::Section,
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

            fn show(&self, at: #gc::inspect::Inspector<'_>) {
                use #gc::inspect::pick::{ViaDebug as _, ViaInspect as _, ViaNothing as _};
                (&&&#gc::inspect::pick::Probe(self)).show(at)
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cycles(tokens: TokenStream) -> syn::Result<()> {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        check_cycles(&parse_manifest(&tokens)?.handlers)
    }

    #[test]
    fn an_unmarked_cycle_away_from_the_first_handler_is_refused_by_name() {
        let checked = cycles(quote! {
            Bouncer {
                handlers {
                    Start => { send Ping },
                    Ping => { send Pong },
                    Pong => { send Ping },
                }
            }
        });

        let error = checked.expect_err("Ping -> Pong -> Ping has no `loop`").to_string();
        assert!(error.contains("`Ping` -> `Pong` -> `Ping`"), "{error}");
    }

    #[test]
    fn a_message_that_sends_itself_without_loop_is_refused() {
        let checked = cycles(quote! {
            Ticker {
                handlers { Tick => { send Tick } }
            }
        });

        assert!(checked.is_err());
    }

    #[test]
    fn a_cycle_with_loop_on_one_edge_is_allowed() {
        let checked = cycles(quote! {
            Bouncer {
                handlers {
                    Start => { send Ping },
                    Ping => { send Pong },
                    Pong => { send Ping loop },
                    Tick => { task Tick loop },
                }
            }
        });

        assert!(checked.is_ok(), "{checked:?}");
    }

    fn requests(tokens: TokenStream) -> syn::Result<()> {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        check_requests(&parse_manifest(&tokens)?.handlers)
    }

    #[test]
    fn a_request_handled_beside_its_payload_with_edges_is_refused() {
        let checked = requests(quote! {
            Processes {
                handlers {
                    RpcRequest<Kill> => { send Killed },
                    Kill,
                    Killed,
                }
            }
        });

        let error = checked.expect_err("Kill would send what the request declares").to_string();
        assert!(error.contains("`Kill`"), "{error}");
    }

    #[test]
    fn a_request_handled_beside_its_payload_without_edges_is_allowed() {
        let checked = requests(quote! {
            Processes {
                handlers { RpcRequest<Kill>, Kill }
            }
        });

        assert!(checked.is_ok(), "{checked:?}");
    }

    #[test]
    fn two_paths_to_one_message_are_not_a_cycle() {
        let checked = cycles(quote! {
            Fork {
                handlers {
                    Start => { send Left, send Right },
                    Left => { send End },
                    Right => { send End },
                    End,
                }
            }
        });

        assert!(checked.is_ok(), "{checked:?}");
    }
}
