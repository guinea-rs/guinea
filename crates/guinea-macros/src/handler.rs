use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    Error, FnArg, GenericArgument, ItemFn, PatType, PathArguments, ReturnType, Result, Type,
    spanned::Spanned,
};

pub fn generate_standalone_handler(item: ItemFn) -> TokenStream {
    expand_handler(item).unwrap_or_else(|err| err.to_compile_error().into())
}

fn expand_handler(mut item: ItemFn) -> Result<TokenStream> {
    let gc = crate::paths::core();
    let is_async = item.sig.asyncness.is_some();

    let signature = item.sig.span();
    let (actor_ty, actor_name) = if is_async {
        extract_actor_from_async_ctx(item.sig.inputs.get(0), signature)?
    } else {
        extract_actor_from_ref(item.sig.inputs.get(0), signature)?
    };
    let actor_ty = actor_ty.clone();
    let actor_ty = &actor_ty;

    let msg_ty = extract_msg_info(item.sig.inputs.get(1), signature)?.0.clone();
    let msg_ty = &msg_ty;

    let takes_cx = !is_async && item.sig.inputs.len() == 3;
    if takes_cx {
        spell_out_cx(&mut item, actor_ty, msg_ty);
    }

    let fn_name = &item.sig.ident;
    let inputs = &item.sig.inputs;
    let cx = if takes_cx {
        quote!(__guinea_cx)
    } else {
        quote!(_)
    };
    let passed_cx = takes_cx.then(|| quote!(, __guinea_cx));

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
        (false, None) => {
            if !(2..=3).contains(&inputs.len()) {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "a handler for `{actor_name}` takes the actor, the message, and - when it sends, publishes or spawns - its `Cx`: (this: &mut {actor_name}, msg: Msg) or (this: &mut {actor_name}, msg: Msg, cx: Cx<{actor_name}, Msg>)"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle(&mut self, __guinea_msg: #msg_ty, #cx: #gc::actor::Cx<Self, #msg_ty>) {
                        #fn_name(self, __guinea_msg #passed_cx);
                    }
                }
            }
        }

        (false, Some(_)) => {
            if !(2..=3).contains(&inputs.len()) {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "a handler answering a request for `{actor_name}` takes the actor, the request, and optionally its `Cx`: (this: &mut {actor_name}, req: Req) or (this: &mut {actor_name}, req: Req, cx: Cx<{actor_name}, Req>)"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::event_bus::rpc::RpcHandler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle_rpc(
                        &mut self,
                        __guinea_msg: #msg_ty,
                        #cx: #gc::actor::Cx<Self, #msg_ty>,
                    ) -> #gc::actor::event_bus::rpc::Reply<
                        <#msg_ty as #gc::actor::event_bus::rpc::RpcCall>::Response,
                    > {
                        ::core::convert::Into::into(#fn_name(self, __guinea_msg #passed_cx))
                    }
                }
            }
        }

        (true, None) => {
            if inputs.len() != 2 {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "an async handler for `{actor_name}` takes its context and the message: (cx: AsyncContext<{actor_name}>, msg: Msg)"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle(&mut self, __guinea_msg: #msg_ty, __guinea_cx: #gc::actor::Cx<Self, #msg_ty>) {
                        let __guinea_actx = __guinea_cx.async_ctx();
                        __guinea_cx.task().detached().spawn_with(move |_| async move {
                            #fn_name(__guinea_actx, __guinea_msg).await;
                        });
                    }
                }
            }
        }

        (true, Some(ret_ty)) => {
            if inputs.len() != 2 {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "an async handler answering a request for `{actor_name}` takes its context and the request: (cx: AsyncContext<{actor_name}>, req: Req)"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#gc::actor::event_bus::RpcRequest<#msg_ty>> for #actor_ty #where_clause {
                    #declared

                    const ANSWERS: bool = true;

                    fn handle(
                        &mut self,
                        __guinea_msg: #gc::actor::event_bus::RpcRequest<#msg_ty>,
                        __guinea_cx: #gc::actor::Cx<Self, #gc::actor::event_bus::RpcRequest<#msg_ty>>,
                    ) {
                        let __guinea_actx = __guinea_cx.async_ctx();
                        let (__guinea_payload, __guinea_reply_to) = __guinea_msg.into_parts();
                        #gc::actor::event_bus::ReplyTo::<#ret_ty>::later(
                            __guinea_reply_to,
                            async move { #fn_name(__guinea_actx, __guinea_payload).await },
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

fn extract_msg_info(arg: Option<&FnArg>, signature: proc_macro2::Span) -> Result<(&Type, String)> {
    let arg = arg.ok_or_else(|| {
        Error::new(signature, "a handler takes the message it handles as its second argument")
    })?;
    if let FnArg::Typed(PatType { ty, .. }) = arg {
        Ok((ty.as_ref(), type_to_string(ty)))
    } else {
        Err(Error::new(arg.span(), "expected the message, `msg: Msg`"))
    }
}

fn extract_actor_from_ref(arg: Option<&FnArg>, signature: proc_macro2::Span) -> Result<(&Type, String)> {
    let arg = arg.ok_or_else(|| {
        Error::new(signature, "a handler takes its actor first: `this: &mut Actor`")
    })?;
    if let FnArg::Typed(PatType { ty, .. }) = arg {
        if let Type::Reference(tr) = ty.as_ref() {
            let inner = tr.elem.as_ref();
            return Ok((inner, type_to_string(inner)));
        }
    }
    Err(Error::new(
        arg.span(),
        "a handler takes its actor first: `this: &mut Actor`, or `this: &Actor`",
    ))
}

fn extract_actor_from_async_ctx(
    arg: Option<&FnArg>,
    signature: proc_macro2::Span,
) -> Result<(&Type, String)> {
    let arg = arg.ok_or_else(|| {
        Error::new(signature, "an async handler takes its context first: `cx: AsyncContext<Actor>`")
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
                    "name the actor: `AsyncContext<Actor>`",
                ));
            }
        }
    }
    Err(Error::new(
        arg.span(),
        "an async handler takes its context first: `cx: AsyncContext<Actor>`",
    ))
}
