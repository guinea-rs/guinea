use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{quote, quote_spanned};
use syn::{
    Error, FnArg, GenericArgument, ItemFn, PatType, PathArguments, ReturnType, Result, Type,
    spanned::Spanned,
};

pub fn generate_standalone_handler(item: ItemFn) -> TokenStream {
    expand_handler(item).unwrap_or_else(|err| err.to_compile_error().into())
}

/// Where `guinea-core` is, from wherever this is being expanded: the crate
/// itself, a direct dependency, or the facade's `core`.
pub(crate) fn guinea_core_crate_path() -> proc_macro2::TokenStream {
    match crate_name("guinea-core") {
        Ok(FoundCrate::Itself) => return quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            return quote!(::#ident);
        }
        Err(_) => {}
    }

    match crate_name("guinea") {
        Ok(FoundCrate::Itself) => quote!(crate::core),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident::core)
        }
        Err(_) => quote!(::guinea::core),
    }
}

fn expand_handler(mut item: ItemFn) -> Result<TokenStream> {
    let gc = guinea_core_crate_path();
    let is_async = item.sig.asyncness.is_some();

    let (actor_ty, actor_name) = if is_async {
        extract_actor_from_async_ctx(item.sig.inputs.get(0))?
    } else {
        extract_actor_from_ref(item.sig.inputs.get(0))?
    };
    let actor_ty = actor_ty.clone();
    let actor_ty = &actor_ty;

    let msg_ty = if is_async {
        let (ty, _) = extract_msg_info(item.sig.inputs.get(1))?;
        ty.clone()
    } else {
        extract_sync_msg(item.sig.inputs.get(1))?
    };
    let msg_ty = &msg_ty;

    let takes_cx = !is_async && item.sig.inputs.len() == 3;
    if takes_cx {
        spell_out_cx(&mut item, actor_ty, msg_ty);
    }

    let fn_name = &item.sig.ident;
    let inputs = &item.sig.inputs;
    let cx = if takes_cx {
        quote!(cx)
    } else {
        quote!(_cx)
    };
    let passed_cx = takes_cx.then(|| quote!(, cx));

    // A return type is the signal that this handler answers an
    // `AsyncBus::request` rather than reacting to a fire-and-forget event.
    // The generated impl is the only code path that can call
    // `AsyncBus::reply` for this message, so "replies exactly once" holds
    // by construction regardless of which of the four branches below fires.
    let ret_ty = match &item.sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(ty),
    };

    let trait_generics = item.sig.generics.clone();
    let (impl_generics, _, where_clause) = trait_generics.split_for_impl();

    let place = quote_spanned! {fn_name.span()=>
        #gc::actor::shape::Declared {
            file: ::core::file!(),
            line: ::core::line!(),
            column: ::core::column!(),
            crate_dir: ::core::env!("CARGO_MANIFEST_DIR"),
        }
    };
    let declared = quote! {
        const DECLARED: ::core::option::Option<#gc::actor::shape::Declared> =
            ::core::option::Option::Some(#place);
    };

    let trait_impl = match (is_async, ret_ty) {
        // Fire-and-forget sync handler.
        (false, None) => {
            if !(2..=3).contains(&inputs.len()) {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "a handler for '{actor_name}' takes the actor, the message, and - when it sends, publishes or spawns - its `Cx`: (this: &mut {actor_name}, msg: Msg) or (this: &mut {actor_name}, msg: Msg, cx: Cx<{actor_name}, Msg>)"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle(&mut self, msg: #msg_ty, #cx: #gc::actor::Cx<Self, #msg_ty>) {
                        #fn_name(self, msg #passed_cx);
                    }
                }
            }
        }

        // Sync RPC handler - `RpcHandler<#msg_ty>`'s blanket `Handler<RpcRequest<#msg_ty>>`
        // impl (in guinea-core) replies with what this returns: the reply
        // itself, or a `Reply` that may come later.
        (false, Some(_)) => {
            if !(2..=3).contains(&inputs.len()) {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "an RPC handler for '{actor_name}' takes the actor, the request, and optionally its `Cx`: (this: &mut {actor_name}, req: Req) or (this: &mut {actor_name}, req: Req, cx: Cx<{actor_name}, Req>)"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::event_bus::rpc::RpcHandler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle_rpc(
                        &mut self,
                        msg: #msg_ty,
                        #cx: #gc::actor::Cx<Self, #msg_ty>,
                    ) -> #gc::actor::event_bus::rpc::Reply<
                        <#msg_ty as #gc::actor::event_bus::rpc::RpcCall>::Response,
                    > {
                        ::core::convert::Into::into(#fn_name(self, msg #passed_cx))
                    }
                }
            }
        }

        // Fire-and-forget async handler - unchanged from before the RPC
        // heuristic existed.
        (true, None) => {
            if inputs.len() != 2 {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "Async handler for actor '{}' must have exactly 2 arguments: (ctx: AsyncContext<{}>, msg: _)",
                        actor_name, actor_name
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle(&mut self, msg: #msg_ty, cx: #gc::actor::Cx<Self, #msg_ty>) {
                        let actx = cx.async_ctx();
                        // `_with`, so the body is left to finish once it has
                        // been told: it holds the same token through its
                        // `AsyncContext`, and that promise is only true if
                        // nothing drops it at the next await.
                        cx.spawn_bg_detached_with(move |_gone| async move {
                            #fn_name(actx, msg).await;
                        });
                    }
                }
            }
        }

        // Async RPC handler - the reply only becomes available after
        // `.await`, so `RpcHandler` (sync-only) doesn't fit. Implement
        // `Handler<RpcRequest<#msg_ty>>` directly instead: unwrap the
        // envelope up front, spawn the body, and reply with whatever it
        // returns once it resolves. The user's fn body never sees
        // `correlation_id` and has no way to call `reply` itself - this
        // generated glue is the only thing that can, same "exactly once"
        // guarantee as the sync branch, just paid for with generated code
        // instead of the trait system (async traits can't express "you
        // must eventually produce exactly one value" the way a plain
        // return type does).
        (true, Some(ret_ty)) => {
            if inputs.len() != 2 {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "Async RPC handler for actor '{}' must have exactly 2 arguments: (ctx: AsyncContext<{}>, req: _)",
                        actor_name, actor_name
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#gc::actor::event_bus::RpcRequest<#msg_ty>> for #actor_ty #where_clause {
                    #declared

                    const ANSWERS: bool = true;

                    fn handle(
                        &mut self,
                        msg: #gc::actor::event_bus::RpcRequest<#msg_ty>,
                        cx: #gc::actor::Cx<Self, #gc::actor::event_bus::RpcRequest<#msg_ty>>,
                    ) {
                        let actx = cx.async_ctx();
                        let payload = msg.payload;
                        #gc::actor::event_bus::AsyncBus::spawn_reply::<#ret_ty, _>(
                            msg.correlation_id,
                            msg.chain,
                            async move { #fn_name(actx, payload).await },
                        );
                    }
                }
            }
        }
    };

    Ok(TokenStream::from(quote! {
        #item

        #trait_impl
    }))
}

fn type_to_string(ty: &Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}

/// A bare `Cx` in a handler's signature, written out: the actor and the
/// message are the two arguments before it, and saying them again is noise.
/// `Cx<A, M>` spelled in full is left as it is.
fn spell_out_cx(item: &mut ItemFn, actor: &Type, msg: &Type) {
    let Some(FnArg::Typed(arg)) = item.sig.inputs.get_mut(2) else {
        return;
    };
    let Type::Path(path) = arg.ty.as_mut() else {
        return;
    };
    let Some(last) = path.path.segments.last_mut() else {
        return;
    };

    if last.ident == "Cx" && last.arguments.is_empty() {
        last.arguments = PathArguments::AngleBracketed(syn::parse_quote!(<#actor, #msg>));
    }
}

/// The message type is the second argument's: the handler takes the message
/// itself, destructured or not.
fn extract_sync_msg(arg: Option<&FnArg>) -> Result<Type> {
    let (ty, _) = extract_msg_info(arg)?;

    let is_context = matches!(
        ty,
        Type::Path(path) if path.path.segments.last().is_some_and(|last| last.ident == "Context")
    );
    if is_context {
        return Err(Error::new(
            ty.span(),
            "a handler takes the message itself now, and `Cx` only when it needs one: `fn h(this: &mut Actor, msg: Msg)` or `fn h(this: &mut Actor, msg: Msg, cx: Cx<Actor, Msg>)`",
        ));
    }

    Ok(ty.clone())
}

fn extract_msg_info(arg: Option<&FnArg>) -> Result<(&Type, String)> {
    let arg =
        arg.ok_or_else(|| Error::new(proc_macro2::Span::call_site(), "Missing message argument"))?;
    if let FnArg::Typed(PatType { ty, .. }) = arg {
        Ok((ty.as_ref(), type_to_string(ty)))
    } else {
        Err(Error::new(
            arg.span(),
            "Expected a typed argument for message",
        ))
    }
}

fn extract_actor_from_ref(arg: Option<&FnArg>) -> Result<(&Type, String)> {
    let arg =
        arg.ok_or_else(|| Error::new(proc_macro2::Span::call_site(), "Missing actor argument"))?;
    if let FnArg::Typed(PatType { ty, .. }) = arg {
        if let Type::Reference(tr) = ty.as_ref() {
            let inner = tr.elem.as_ref();
            return Ok((inner, type_to_string(inner)));
        }
    }
    Err(Error::new(
        arg.span(),
        "First argument of a sync handler must be '&ActorType' or '&mut ActorType'",
    ))
}

fn extract_actor_from_async_ctx(arg: Option<&FnArg>) -> Result<(&Type, String)> {
    let arg = arg.ok_or_else(|| {
        Error::new(
            proc_macro2::Span::call_site(),
            "Missing AsyncContext argument",
        )
    })?;
    if let FnArg::Typed(PatType { ty, .. }) = arg {
        if let Type::Path(tp) = ty.as_ref() {
            let last_segment = tp
                .path
                .segments
                .last()
                .ok_or_else(|| Error::new(arg.span(), "Invalid path for AsyncContext"))?;

            if last_segment.ident == "AsyncContext" {
                if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Ok((inner_ty, type_to_string(inner_ty)));
                    }
                }
                return Err(Error::new(
                    arg.span(),
                    "AsyncContext must specify the Actor type in angle brackets: AsyncContext<MyActor>",
                ));
            }
        }
    }
    Err(Error::new(
        arg.span(),
        "First argument of an async handler must be 'AsyncContext<ActorType>'",
    ))
}
