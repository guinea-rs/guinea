use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    Error, FnArg, GenericArgument, ItemFn, PatType, PathArguments, ReturnType, Result, Type,
    spanned::Spanned,
};

pub fn generate_standalone_handler(item: ItemFn) -> TokenStream {
    expand_handler(item).unwrap_or_else(|err| err.to_compile_error().into())
}

fn expand_handler(item: ItemFn) -> Result<TokenStream> {
    let gc = crate::paths::core();
    let is_async = item.sig.asyncness.is_some();
    let signature = item.sig.span();

    let (actor_ty, msg_ty) = if is_async {
        let actor = extract_actor_from_async_ctx(item.sig.inputs.get(0), signature)?.0.clone();
        let msg = extract_msg_info(item.sig.inputs.get(1), signature)?.0.clone();
        (actor, msg)
    } else {
        handed(item.sig.inputs.first()).ok_or_else(|| {
            Error::new(
                signature,
                "a handler takes what it is handed, taken apart: `In(this, msg, cx): In<Actor, Msg>`",
            )
        })?
    };
    let actor_ty = &actor_ty;
    let msg_ty = &msg_ty;
    let actor_name = type_to_string(actor_ty);

    let fn_name = &item.sig.ident;
    let inputs = &item.sig.inputs;

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
            if inputs.len() != 1 {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "a handler for `{actor_name}` takes one argument, what it is handed: `In(this, msg, cx): In<{actor_name}, Msg>`"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::actor::Handler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle(__guinea_handed: #gc::actor::In<'_, Self, #msg_ty>) {
                        #fn_name(__guinea_handed);
                    }
                }
            }
        }

        (false, Some(_)) => {
            if inputs.len() != 1 {
                return Err(Error::new(
                    item.sig.span(),
                    format!(
                        "a handler answering a request for `{actor_name}` takes one argument, what it is handed: `In(this, req, cx): In<{actor_name}, Req>`"
                    ),
                ));
            }
            quote! {
                impl #impl_generics #gc::__private::RpcHandler<#msg_ty> for #actor_ty #where_clause {
                    #declared

                    fn handle_rpc(
                        __guinea_handed: #gc::actor::In<'_, Self, #msg_ty>,
                    ) -> #gc::actor::event_bus::Reply<
                        <#msg_ty as #gc::actor::event_bus::RpcCall>::Response,
                    > {
                        ::core::convert::Into::into(#fn_name(__guinea_handed))
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

                    fn handle(
                        #gc::actor::In(_, __guinea_msg, __guinea_cx): #gc::actor::In<'_, Self, #msg_ty>,
                    ) {
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
                        #gc::actor::In(_, __guinea_msg, __guinea_cx): #gc::actor::In<
                            '_,
                            Self,
                            #gc::actor::event_bus::RpcRequest<#msg_ty>,
                        >,
                    ) {
                        let __guinea_actx = __guinea_cx.async_ctx();
                        let (__guinea_payload, __guinea_reply_to) = __guinea_msg.into_parts();
                        #gc::__private::ReplyTo::<#ret_ty>::later(
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

/// The actor and the message of `In<A, M>`, when that is what the handler
/// takes.
fn handed(arg: Option<&FnArg>) -> Option<(Type, Type)> {
    let Some(FnArg::Typed(PatType { ty, .. })) = arg else {
        return None;
    };
    let Type::Path(path) = ty.as_ref() else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "In" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };

    let mut types = args.args.iter().filter_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    });
    let actor = types.next()?;
    let msg = types.next()?;

    Some((actor, msg))
}

fn type_to_string(ty: &Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
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
