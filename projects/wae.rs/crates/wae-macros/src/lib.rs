//! WAE 过程宏：N-API 函数导出、对象映射与模块注册。
//!
//! 生成代码一律通过门面 crate `::wae` 解析类型与辅助函数。
//!
//! 三个宏分工：
//! - `#[function]`：把 Rust 函数包成 Node-API 回调，并按签名生成类型元数据；
//! - `#[object]`：为字段结构生成 `FromJs` / `ToJs` / `TsType`；
//! - `register_module!`：注册导出表，并挂上 `__waeTypes` 元数据。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Expr, ExprLit, Fields, FnArg, ItemFn, Lit, Meta, MetaNameValue, Pat, ReturnType, Token, Type, TypePath,
    parse_macro_input,
};

/// 把函数包成 Node-API 导出。
///
/// 支持两种签名：
///
/// - 类型化（推荐）：`fn render(options: RenderOptions) -> Result<RenderReport>`，
///   参数与返回值经 `FromJs` / `ToJs` 转换，并参与 TypeScript 声明生成；
/// - 旧形态：`fn version(ctx: CallContext) -> Result<JsValue>`，参数面按 `unknown` 处理。
///
/// 可选参数：`js_name = "..."`（默认用 Rust 函数标识符原文）。
#[proc_macro_attribute]
pub fn function(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match parse_function_args(attr) {
        Ok(value) => value,
        Err(error) => return error.to_compile_error().into(),
    };
    let input = parse_macro_input!(item as ItemFn);
    match expand_function(args, input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// 让字段结构在 JS 边界上可读可写。
///
/// 默认把 Rust `snake_case` 字段名映射为 JS `camelCase`；需要精确命名时在字段上写
/// `#[js_name = "..."])`。`Option<T>` 字段在 JS 侧可省略，输出时为空则不写该键。
#[proc_macro_attribute]
pub fn object(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "`#[object]` 不接受参数").to_compile_error().into();
    }
    let input = parse_macro_input!(item as DeriveInput);
    match expand_object(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// 生成 `napi_register_module_v1`，参数为已用 [`function`] 标注的 Rust 函数名列表。
///
/// ```ignore
/// wae::register_module![scan, version, open_gui];
/// ```
#[proc_macro]
pub fn register_module(input: TokenStream) -> TokenStream {
    let tokens: TokenStream2 = input.into();
    let (functions, classes) = match parse_registration(tokens) {
        Ok(value) => value,
        Err(error) => return error.to_compile_error().into(),
    };
    if functions.is_empty() && classes.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "register_module! 至少需要一个导出函数或类").to_compile_error().into();
    }

    let entries = functions.iter().map(|id| {
        let const_name = format_ident!("WAE_NAPI_EXPORT_{}", id.to_string().to_ascii_uppercase());
        quote! { #const_name }
    });
    let metas = functions.iter().map(|id| {
        let meta_name = format_ident!("wae_meta_{}", id);
        quote! { #meta_name }
    });
    let class_entries = classes.iter().map(|id| {
        let const_name = format_ident!("WAE_NAPI_CLASS_{}", id.to_string().to_ascii_uppercase());
        quote! { #const_name }
    });
    let class_metas = classes.iter().map(|id| {
        let meta_name = format_ident!("wae_meta_class_{}", snake_case(&id.to_string()));
        quote! { #meta_name }
    });

    quote! {
        /// Node 加载 addon 时调用的入口。
        ///
        /// # Safety
        /// `env` / `exports` 必须是当前加载有效的 Node-API 句柄。
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn napi_register_module_v1(
            env: ::wae::napi_env,
            exports: ::wae::napi_value,
        ) -> ::wae::napi_value {
            ::wae::setup();
            match ::wae::register_exports_with_types(
                env,
                exports,
                &[
                    #(#entries),*
                ],
                &[
                    #(#class_entries),*
                ],
                &[
                    #(#metas),*
                ],
                &[
                    #(#class_metas),*
                ],
            ) {
                Ok(value) => value,
                Err(error) => {
                    ::wae::Env::from_raw(env).throw_error(error.message());
                    exports
                }
            }
        }
    }
    .into()
}

/// 把 `fn..., ...; Class, ...` 拆成函数表与类表。
fn parse_registration(tokens: TokenStream2) -> syn::Result<(Vec<syn::Ident>, Vec<syn::Ident>)> {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let mut current: Vec<proc_macro2::TokenTree> = Vec::new();
    let mut in_classes = false;
    for token in tokens {
        if let proc_macro2::TokenTree::Punct(punct) = &token {
            if punct.as_char() == ';' {
                if !current.is_empty() {
                    let ident = ident_from_group(&current)?;
                    current.clear();
                    functions.push(ident);
                }
                in_classes = true;
                continue;
            }
            if punct.as_char() == ',' {
                let ident = ident_from_group(&current)?;
                current.clear();
                if in_classes {
                    classes.push(ident)
                }
                else {
                    functions.push(ident)
                }
                continue;
            }
        }
        current.push(token);
    }
    if !current.is_empty() {
        let ident = ident_from_group(&current)?;
        if in_classes { classes.push(ident) } else { functions.push(ident) }
    }
    Ok((functions, classes))
}

fn ident_from_group(tokens: &[proc_macro2::TokenTree]) -> syn::Result<syn::Ident> {
    let mut iter = tokens.iter();
    let Some(proc_macro2::TokenTree::Ident(ident)) = iter.next()
    else {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), "register_module! 只接受标识符"));
    };
    if let Some(extra) = iter.next() {
        return Err(syn::Error::new_spanned(extra, "register_module! 只接受标识符"));
    }
    Ok(ident.clone())
}

struct FunctionArgs {
    js_name: Option<String>,
    is_async: bool,
}

fn parse_function_args(attr: TokenStream) -> syn::Result<FunctionArgs> {
    let tokens: TokenStream2 = attr.into();
    if tokens.is_empty() {
        return Ok(FunctionArgs { js_name: None, is_async: false });
    }
    let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<FunctionArgs> {
        let mut js_name = None;
        let mut is_async = false;
        loop {
            if input.peek(Token![async]) {
                input.parse::<Token![async]>()?;
                is_async = true;
            }
            else {
                let ident: syn::Ident = input.parse()?;
                if ident != "js_name" {
                    return Err(syn::Error::new(ident.span(), "仅支持 `async` 与 `js_name = \"...\"`"));
                }
                input.parse::<Token![=]>()?;
                let value: syn::LitStr = input.parse()?;
                js_name = Some(value.value());
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
        }
        Ok(FunctionArgs { js_name, is_async })
    };
    syn::parse::Parser::parse2(parser, tokens)
}

fn expr_to_string(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Str(text), .. }) => Ok(text.value()),
        other => Err(syn::Error::new_spanned(other, "js_name 必须是字符串字面量")),
    }
}

/// 去引用后判断类型路径的最后一段是否为给定名字。
fn type_is(ty: &Type, expected: &str) -> bool {
    match ty {
        Type::Reference(reference) => type_is(&reference.elem, expected),
        Type::Path(path) => path.path.segments.last().is_some_and(|segment| segment.ident == expected),
        _ => false,
    }
}

fn success_type(output: &ReturnType) -> syn::Result<Type> {
    let ReturnType::Type(_, ty) = output
    else {
        return Err(syn::Error::new_spanned(output, "导出函数必须返回 `Result<T>`"));
    };
    let Type::Path(TypePath { path, .. }) = &**ty
    else {
        return Err(syn::Error::new_spanned(ty, "导出函数必须返回 `Result<T>`"));
    };
    let Some(segment) = path.segments.last()
    else {
        return Err(syn::Error::new_spanned(ty, "导出函数必须返回 `Result<T>`"));
    };
    if segment.ident != "Result" {
        return Err(syn::Error::new_spanned(ty, "导出函数必须返回 `Result<T>`"));
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
    else {
        return Err(syn::Error::new_spanned(ty, "`Result` 缺少返回值类型"));
    };
    match arguments.args.first() {
        Some(syn::GenericArgument::Type(inner)) => Ok(inner.clone()),
        _ => Err(syn::Error::new_spanned(ty, "`Result` 缺少返回值类型")),
    }
}

/// `Option<T>` 的内层类型。
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(TypePath { path, .. }) = ty
    else {
        return None;
    };
    let segment = path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
    else {
        return None;
    };
    match arguments.args.first() {
        Some(syn::GenericArgument::Type(inner)) => Some(inner),
        _ => None,
    }
}

struct DataParam {
    name: syn::Ident,
    ty: Type,
    optional: bool,
}

fn expand_function(args: FunctionArgs, input: ItemFn) -> syn::Result<TokenStream2> {
    let signature = &input.sig;
    if signature.asyncness.is_some() {
        return Err(syn::Error::new_spanned(signature, "异步导出尚未支持（P2 才接线）"));
    }
    if signature.generics.params.iter().next().is_some() {
        return Err(syn::Error::new_spanned(&signature.generics, "导出函数不支持泛型参数"));
    }

    let fn_ident = signature.ident.clone();
    let js_name = args.js_name.unwrap_or_else(|| fn_ident.to_string());
    let is_async = args.is_async;
    let export_fn = format_ident!("wae_napi_export_{}", fn_ident);
    let export_const = format_ident!("WAE_NAPI_EXPORT_{}", fn_ident.to_string().to_ascii_uppercase());
    let meta_fn = format_ident!("wae_meta_{}", fn_ident);
    let decl_fn = format_ident!("wae_decls_{}", fn_ident);
    let vis = input.vis.clone();
    let success = success_type(&signature.output)?;
    let untyped = type_is(&success, "JsValue");

    let mut ctx_param: Option<&FnArg> = None;
    let mut ctx_by_ref = false;
    let mut data: Vec<DataParam> = Vec::new();
    for (index, arg) in signature.inputs.iter().enumerate() {
        let FnArg::Typed(typed) = arg
        else {
            return Err(syn::Error::new_spanned(arg, "导出函数不支持 `self` 参数"));
        };
        if index == 0 && type_is(&typed.ty, "CallContext") {
            ctx_param = Some(arg);
            ctx_by_ref = matches!(&*typed.ty, Type::Reference(_));
            continue;
        }
        let Pat::Ident(pattern) = &*typed.pat
        else {
            return Err(syn::Error::new_spanned(&typed.pat, "导出函数的参数必须是简单标识符"));
        };
        data.push(DataParam { name: pattern.ident.clone(), ty: (*typed.ty).clone(), optional: option_inner(&typed.ty).is_some() });
    }

    let has_ctx = ctx_param.is_some();
    if is_async && has_ctx {
        return Err(syn::Error::new_spanned(signature, "async 导出不能接收 `CallContext`：工作线程不能触碰 `Env`"));
    }
    let arg_lets = data.iter().enumerate().map(|(index, param)| {
        let name = &param.name;
        let ty = &param.ty;
        let label = format!("argument {index} ({name})");
        if param.optional {
            quote! {
                let #name = (|| -> ::wae::Result<#ty> {
                    match ctx.args().get(#index).copied() {
                        Some(value) => <#ty as ::wae::FromJs>::from_js(value),
                        None => Ok(None),
                    }
                })()
                .map_err(|error: ::wae::Error| error.context(#label))?;
            }
        }
        else {
            quote! {
                let #name = (|| -> ::wae::Result<#ty> {
                    <#ty as ::wae::FromJs>::from_js(ctx.arg(#index)?)
                })()
                .map_err(|error: ::wae::Error| error.context(#label))?;
            }
        }
    });
    let arg_names: Vec<syn::Ident> = data.iter().map(|param| param.name.clone()).collect();
    let call = if has_ctx && ctx_by_ref {
        quote! { #fn_ident(&ctx #(, #arg_names)*) }
    }
    else if has_ctx {
        quote! { #fn_ident(ctx #(, #arg_names)*) }
    }
    else {
        quote! { #fn_ident(#(#arg_names),*) }
    };

    let body = if is_async {
        let task_ident = format_ident!("WaeTask_{}", fn_ident);
        let execute_fn = format_ident!("wae_execute_{}", fn_ident);
        let complete_fn = format_ident!("wae_complete_{}", fn_ident);
        let arg_types: Vec<&Type> = data.iter().map(|param| &param.ty).collect();
        let task_name = format!("wae:{js_name}");
        quote! {{
            #(#arg_lets)*
            fn __wae_assert_send<T: Send>() {}
            __wae_assert_send::<(#(#arg_types,)*)>();
            __wae_assert_send::<#success>();
            let (__promise, __deferred) = ctx.env().promise()?;
            let __task = Box::new(#task_ident {
                args: Some((#(#arg_names,)*)),
                deferred: Some(__deferred),
                work: None,
                result: None,
            });
            let __raw = Box::into_raw(__task);
            let __work = match ctx.env().async_work(#task_name, __raw as *mut ::std::ffi::c_void, #execute_fn, #complete_fn) {
                Ok(work) => work,
                Err(error) => {
                    drop(unsafe { Box::from_raw(__raw) });
                    return Err(error);
                }
            };
            unsafe { (*__raw).work = Some(__work); }
            if let Err(error) = ctx.env().queue_work(__work) {
                ctx.env().delete_work(__work);
                drop(unsafe { Box::from_raw(__raw) });
                return Err(error);
            }
            Ok(__promise)
        }}
    }
    else if untyped && data.is_empty() {
        // 旧形态：参数面由函数自己从 `ctx.args()` 读取，包装层不介入。
        quote! { #fn_ident(ctx) }
    }
    else {
        quote! {{
            #(#arg_lets)*
            let __env = ctx.env();
            let __result = #call;
            ::wae::ToJs::to_js(__result?, __env)
        }}
    };

    let params = data.iter().map(|param| {
        let name = param.name.to_string();
        let ty = option_inner(&param.ty).unwrap_or(&param.ty);
        let optional = param.optional;
        quote! {
            ::wae::ParamMeta { name: #name, ts: <#ty as ::wae::TsType>::ts_type, optional: #optional }
        }
    });
    let declarations = data.iter().map(|param| {
        let ty = &param.ty;
        quote! { <#ty as ::wae::TsType>::declarations(out); }
    });

    let mut output = input;
    output.attrs.retain(|attr| !attr.path().is_ident("function"));

    let async_defs = if is_async {
        let task_ident = format_ident!("WaeTask_{}", fn_ident);
        let execute_fn = format_ident!("wae_execute_{}", fn_ident);
        let complete_fn = format_ident!("wae_complete_{}", fn_ident);
        let arg_types: Vec<&Type> = data.iter().map(|param| &param.ty).collect();
        quote! {
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            struct #task_ident {
                args: Option<(#(#arg_types,)*)>,
                deferred: Option<::wae::napi_deferred>,
                work: Option<::wae::napi_async_work>,
                result: Option<::wae::Result<#success>>,
            }

            #[doc(hidden)]
            unsafe extern "C" fn #execute_fn(_env: ::wae::napi_env, data: *mut ::std::ffi::c_void) {
                // 工作线程：只跑纯 Rust 逻辑，不触碰 `Env` / `JsValue`。
                let task = unsafe { &mut *(data as *mut #task_ident) };
                let Some(args) = task.args.take()
                else {
                    return;
                };
                let (#(#arg_names,)*) = args;
                task.result = Some(#fn_ident(#(#arg_names),*));
            }

            #[doc(hidden)]
            unsafe extern "C" fn #complete_fn(env: ::wae::napi_env, _status: ::wae::napi_status, data: *mut ::std::ffi::c_void) {
                // 主线程：转换返回值、结算 Promise、删除任务。
                let task = unsafe { Box::from_raw(data as *mut #task_ident) };
                let env = ::wae::Env::from_raw(env);
                if let Some(work) = task.work {
                    env.delete_work(work);
                }
                let Some(deferred) = task.deferred
                else {
                    return;
                };
                match task.result {
                    Some(Ok(value)) => match ::wae::ToJs::to_js(value, env) {
                        Ok(value) => {
                            let _ = env.resolve(deferred, value);
                        }
                        Err(error) => {
                            if let Ok(value) = env.error(&error) {
                                let _ = env.reject(deferred, value);
                            }
                        }
                    },
                    Some(Err(error)) => {
                        if let Ok(value) = env.error(&error) {
                            let _ = env.reject(deferred, value);
                        }
                    }
                    None => {}
                }
            }
        }
    }
    else {
        quote! {}
    };

    Ok(quote! {
        #output

        #async_defs
        #[doc(hidden)]
        const #export_const: (
            &'static str,
            unsafe extern "C" fn(::wae::napi_env, ::wae::napi_callback_info) -> ::wae::napi_value,
        ) = (#js_name, #export_fn);

        #[doc(hidden)]
        #vis unsafe extern "C" fn #export_fn(
            env: ::wae::napi_env,
            info: ::wae::napi_callback_info,
        ) -> ::wae::napi_value {
            unsafe {
                ::wae::catch_callback(env, info, |ctx: ::wae::CallContext| #body)
            }
        }

        #[doc(hidden)]
        fn #decl_fn(out: &mut Vec<::wae::TypeDecl>) {
            #(#declarations)*
            <#success as ::wae::TsType>::declarations(out);
        }

        #[doc(hidden)]
        fn #meta_fn() -> ::wae::FunctionMeta {
            ::wae::FunctionMeta {
                js_name: #js_name,
                params: &[ #(#params),* ],
                returns: <#success as ::wae::TsType>::ts_type,
                untyped: #untyped,
                is_async: #is_async,
                declarations: #decl_fn,
            }
        }
    })
}

/// 让 `impl` 块成为可导出的 JS 类。
///
/// 成员标记：`#[constructor]`（恰好一个）、`#[method]`、`#[getter]`。
/// Rust 方法名按 camelCase 映射到 JS；实例保存在 `Box<T>` 里，由 finalizer 回收。
///
/// ```ignore
/// #[class]
/// impl Counter {
///     #[constructor]
///     fn new(start: u32) -> Result<Self> { Ok(Self { value: start }) }
///
///     #[method]
///     fn add(&mut self, n: u32) -> Result<u32> { self.value += n; Ok(self.value) }
///
///     #[getter]
///     fn value(&self) -> Result<u32> { Ok(self.value) }
/// }
/// ```
#[proc_macro_attribute]
pub fn class(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "`#[class]` 不接受参数").to_compile_error().into();
    }
    let input = parse_macro_input!(item as syn::ItemImpl);
    match expand_class(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

enum MemberKind {
    None,
    Constructor,
    Method,
    Getter,
}

struct ClassMember {
    kind: MemberKind,
    ident: syn::Ident,
    js_name: String,
    params: Vec<DataParam>,
    returns: Type,
    mutable: bool,
}

fn expand_class(input: syn::ItemImpl) -> syn::Result<TokenStream2> {
    if input.trait_.is_some() {
        return Err(syn::Error::new_spanned(&input, "`#[class]` 只能用于固有 impl 块"));
    }
    let self_ty = match &*input.self_ty {
        Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.clone()),
        _ => None,
    };
    let Some(self_ty) = self_ty
    else {
        return Err(syn::Error::new_spanned(&input.self_ty, "`#[class]` 需要具名类型"));
    };

    let mut members: Vec<ClassMember> = Vec::new();
    for item in &input.items {
        let syn::ImplItem::Fn(function) = item
        else {
            continue;
        };
        let mut kind = MemberKind::None;
        for attr in &function.attrs {
            if attr.path().is_ident("constructor") {
                kind = MemberKind::Constructor;
            }
            else if attr.path().is_ident("method") {
                kind = MemberKind::Method;
            }
            else if attr.path().is_ident("getter") {
                kind = MemberKind::Getter;
            }
        }
        if matches!(kind, MemberKind::None) {
            continue;
        }
        let success = success_type(&function.sig.output)?;
        let mut params = Vec::new();
        let mut mutable = false;
        for (index, arg) in function.sig.inputs.iter().enumerate() {
            match arg {
                FnArg::Receiver(receiver) => {
                    mutable = receiver.mutability.is_some();
                    if receiver.reference.is_none() {
                        return Err(syn::Error::new_spanned(receiver, "类成员只能以 `&self` 或 `&mut self` 接收"));
                    }
                }
                FnArg::Typed(typed) => {
                    if !matches!(kind, MemberKind::Constructor) && index == 0 {
                        return Err(syn::Error::new_spanned(typed, "类成员缺少 `self` 接收者"));
                    }
                    let Pat::Ident(pattern) = &*typed.pat
                    else {
                        return Err(syn::Error::new_spanned(&typed.pat, "类成员的参数必须是简单标识符"));
                    };
                    params.push(DataParam {
                        name: pattern.ident.clone(),
                        ty: (*typed.ty).clone(),
                        optional: option_inner(&typed.ty).is_some(),
                    });
                }
            }
        }
        members.push(ClassMember {
            kind,
            js_name: camel_case(&function.sig.ident.to_string()),
            ident: function.sig.ident.clone(),
            params,
            returns: success,
            mutable,
        });
    }

    let constructors: Vec<&ClassMember> = members.iter().filter(|member| matches!(member.kind, MemberKind::Constructor)).collect();
    if constructors.len() != 1 {
        return Err(syn::Error::new_spanned(&input, "`#[class]` 需要恰好一个 `#[constructor]`"));
    }
    let constructor = constructors[0];
    for member in &members {
        if matches!(member.kind, MemberKind::Constructor) && member.ident != constructor.ident {
            return Err(syn::Error::new_spanned(&input, "`#[class]` 只能有一个构造器"));
        }
    }

    let js_class = self_ty.to_string();
    let snake = snake_case(&js_class);
    let class_const = format_ident!("WAE_NAPI_CLASS_{}", self_ty.to_string().to_ascii_uppercase());
    let new_fn = format_ident!("wae_new_{}", snake);
    let finalize_fn = format_ident!("wae_finalize_{}", snake);
    let register_fn = format_ident!("wae_register_class_{}", snake);
    let meta_fn = format_ident!("wae_meta_class_{}", snake);
    let decls_fn = format_ident!("wae_class_decls_{}", snake);

    let ctor_ident = &constructor.ident;
    let ctor_args: Vec<syn::Ident> = constructor.params.iter().map(|param| param.name.clone()).collect();
    let ctor_lets = arg_lets(&constructor.params);

    let mut member_fns = Vec::new();
    let mut descriptors = Vec::new();
    let mut method_metas = Vec::new();
    let mut declarations = Vec::new();
    for (index, member) in members.iter().enumerate() {
        if matches!(member.kind, MemberKind::Constructor) {
            continue;
        }
        let fn_ident = format_ident!("wae_member_{}_{}", snake, index);
        let member_ident = &member.ident;
        let args: Vec<syn::Ident> = member.params.iter().map(|param| param.name.clone()).collect();
        let lets = arg_lets(&member.params);
        let returns = &member.returns;
        let receiver = if member.mutable {
            quote! { let __this_ref = unsafe { &mut *(__ptr as *mut #self_ty) }; }
        }
        else {
            quote! { let __this_ref = unsafe { &*(__ptr as *const #self_ty) }; }
        };
        member_fns.push(quote! {
            #[doc(hidden)]
            unsafe extern "C" fn #fn_ident(
                env: ::wae::napi_env,
                info: ::wae::napi_callback_info,
            ) -> ::wae::napi_value {
                unsafe {
                    ::wae::catch_callback(env, info, |ctx: ::wae::CallContext| {
                        let __this = ctx.this()?;
                        let __ptr = ctx.env().unwrap(__this)?;
                        #receiver
                        #(#lets)*
                        let __value = __this_ref.#member_ident(#(#args),*)?;
                        ::wae::ToJs::to_js(__value, ctx.env())
                    })
                }
            }
        });
        let c_name = member.js_name.clone();
        let js_name_lit = member.js_name.clone();
        let descriptor = if matches!(member.kind, MemberKind::Getter) {
            declarations.push(quote! { <#returns as ::wae::TsType>::declarations(__out); });
            method_metas.push(quote! {
                ::wae::MethodMeta {
                    js_name: #js_name_lit,
                    params: &[],
                    returns: <#returns as ::wae::TsType>::ts_type,
                    kind: ::wae::MethodKind::Getter,
                }
            });
            quote! {
                ::wae::napi_property_descriptor {
                    utf8name: concat!(#c_name, "\0").as_ptr() as *const ::std::os::raw::c_char,
                    name: ::std::ptr::null_mut(),
                    method: None,
                    getter: Some(#fn_ident),
                    setter: None,
                    value: ::std::ptr::null_mut(),
                    attributes: 5,
                    data: ::std::ptr::null_mut(),
                }
            }
        }
        else {
            let params = member.params.iter().map(|param| {
                let name = param.name.to_string();
                let ty = option_inner(&param.ty).unwrap_or(&param.ty);
                let optional = param.optional;
                quote! { ::wae::ParamMeta { name: #name, ts: <#ty as ::wae::TsType>::ts_type, optional: #optional } }
            });
            for param in &member.params {
                let ty = &param.ty;
                declarations.push(quote! { <#ty as ::wae::TsType>::declarations(__out); });
            }
            declarations.push(quote! { <#returns as ::wae::TsType>::declarations(__out); });
            method_metas.push(quote! {
                ::wae::MethodMeta {
                    js_name: #js_name_lit,
                    params: &[ #(#params),* ],
                    returns: <#returns as ::wae::TsType>::ts_type,
                    kind: ::wae::MethodKind::Method,
                }
            });
            quote! {
                ::wae::napi_property_descriptor {
                    utf8name: concat!(#c_name, "\0").as_ptr() as *const ::std::os::raw::c_char,
                    name: ::std::ptr::null_mut(),
                    method: Some(#fn_ident),
                    getter: None,
                    setter: None,
                    value: ::std::ptr::null_mut(),
                    attributes: 5,
                    data: ::std::ptr::null_mut(),
                }
            }
        };
        descriptors.push(descriptor);
    }

    let ctor_params = constructor.params.iter().map(|param| {
        let name = param.name.to_string();
        let ty = option_inner(&param.ty).unwrap_or(&param.ty);
        let optional = param.optional;
        quote! { ::wae::ParamMeta { name: #name, ts: <#ty as ::wae::TsType>::ts_type, optional: #optional } }
    });
    for param in &constructor.params {
        let ty = &param.ty;
        declarations.push(quote! { <#ty as ::wae::TsType>::declarations(__out); });
    }

    // 去掉成员标记，剩下的 `impl` 块原样保留。
    let mut cleaned = input.clone();
    for item in &mut cleaned.items {
        if let syn::ImplItem::Fn(function) = item {
            function
                .attrs
                .retain(|attr| !(attr.path().is_ident("constructor") || attr.path().is_ident("method") || attr.path().is_ident("getter")));
        }
    }
    let property_count = descriptors.len();

    Ok(quote! {
        #cleaned

        #[doc(hidden)]
        unsafe extern "C" fn #finalize_fn(_env: ::wae::napi_env, data: *mut ::std::ffi::c_void, _hint: *mut ::std::ffi::c_void) {
            if !data.is_null() {
                drop(unsafe { Box::from_raw(data as *mut #self_ty) });
            }
        }

        #[doc(hidden)]
        unsafe extern "C" fn #new_fn(
            env: ::wae::napi_env,
            info: ::wae::napi_callback_info,
        ) -> ::wae::napi_value {
            unsafe {
                ::wae::catch_callback(env, info, |ctx: ::wae::CallContext| {
                    if !ctx.is_constructor() {
                        return Err(::wae::Error::invalid_input(concat!(#js_class, " must be called with `new`")));
                    }
                    let __this = ctx.this()?;
                    #(#ctor_lets)*
                    let __value = <#self_ty>::#ctor_ident(#(#ctor_args),*)?;
                    let __boxed = Box::into_raw(Box::new(__value));
                    ctx.env().wrap(__this, __boxed as *mut ::std::ffi::c_void, #finalize_fn)?;
                    Ok(__this)
                })
            }
        }

        #(#member_fns)*

        #[doc(hidden)]
        pub const #class_const: (&'static str, unsafe extern "C" fn(::wae::napi_env, ::wae::napi_value) -> ::wae::napi_status) =
            (#js_class, #register_fn);

        #[doc(hidden)]
        pub unsafe extern "C" fn #register_fn(env: ::wae::napi_env, exports: ::wae::napi_value) -> ::wae::napi_status {
            let __env = ::wae::Env::from_raw(env);
            let __properties: [::wae::napi_property_descriptor; #property_count] = [ #(#descriptors),* ];
            match __env.define_class(#js_class, Some(#new_fn), &__properties) {
                Ok(__class) => match ::wae::JsValue::from_raw(__env, exports).set_named(#js_class, __class) {
                    Ok(()) => 0,
                    Err(_) => 1,
                },
                Err(_) => 1,
            }
        }

        #[doc(hidden)]
        fn #decls_fn(__out: &mut Vec<::wae::TypeDecl>) {
            #(#declarations)*
        }

        #[doc(hidden)]
        fn #meta_fn() -> ::wae::ClassMeta {
            ::wae::ClassMeta {
                js_name: #js_class,
                constructor: &[ #(#ctor_params),* ],
                methods: &[ #(#method_metas),* ],
                declarations: #decls_fn,
            }
        }
    })
}

/// 类成员的参数转换；与 `#[function]` 同形。
fn arg_lets(params: &[DataParam]) -> Vec<TokenStream2> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let name = &param.name;
            let ty = &param.ty;
            let label = format!("argument {index} ({name})");
            if param.optional {
                quote! {
                    let #name = (|| -> ::wae::Result<#ty> {
                        match ctx.args().get(#index).copied() {
                            Some(value) => <#ty as ::wae::FromJs>::from_js(value),
                            None => Ok(None),
                        }
                    })()
                    .map_err(|error: ::wae::Error| error.context(#label))?;
                }
            }
            else {
                quote! {
                    let #name = (|| -> ::wae::Result<#ty> {
                        <#ty as ::wae::FromJs>::from_js(ctx.arg(#index)?)
                    })()
                    .map_err(|error: ::wae::Error| error.context(#label))?;
                }
            }
        })
        .collect()
}

fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for character in name.chars() {
        if character == '_' {
            upper = true;
            continue;
        }
        if upper {
            out.extend(character.to_uppercase());
            upper = false;
        }
        else {
            out.push(character);
        }
    }
    out
}

fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(character.to_lowercase());
        }
        else {
            out.push(character);
        }
    }
    out
}

struct ObjectField {
    ident: syn::Ident,
    ty: Type,
    js_name: String,
    optional: bool,
}

fn expand_object(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    if input.generics.params.iter().next().is_some() {
        return Err(syn::Error::new_spanned(&input.generics, "`#[object]` 不支持泛型结构"));
    }
    let Data::Struct(data) = &input.data
    else {
        return Err(syn::Error::new_spanned(&input, "`#[object]` 只能用于结构体"));
    };
    let Fields::Named(fields) = &data.fields
    else {
        return Err(syn::Error::new_spanned(&input, "`#[object]` 需要具名字段"));
    };

    let mut collected = Vec::with_capacity(fields.named.len());
    for field in &fields.named {
        let ident = field.ident.clone().expect("named field");
        let mut js_name = camel_case(&ident.to_string());
        for attr in &field.attrs {
            if attr.path().is_ident("js_name") {
                let Meta::NameValue(MetaNameValue { value, .. }) = &attr.meta
                else {
                    return Err(syn::Error::new_spanned(attr, "`#[js_name = \"...\"]` 需要字符串字面量"));
                };
                js_name = expr_to_string(value)?;
            }
        }
        collected.push(ObjectField { ident, ty: field.ty.clone(), js_name, optional: option_inner(&field.ty).is_some() });
    }

    let mut cleaned = input.clone();
    if let Data::Struct(data) = &mut cleaned.data
        && let Fields::Named(fields) = &mut data.fields
    {
        for field in &mut fields.named {
            field.attrs.retain(|attr| !attr.path().is_ident("js_name"));
        }
    }

    let from_fields = collected.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        let js_name = &field.js_name;
        let label = format!("field `{js_name}`");
        quote! {
            #ident: (|| -> ::wae::Result<#ty> {
                <#ty as ::wae::FromJs>::from_js(__value.get_named(#js_name)?)
            })()
            .map_err(|error: ::wae::Error| error.context(#label))?,
        }
    });

    let to_fields = collected.iter().map(|field| {
        let ident = &field.ident;
        let js_name = &field.js_name;
        if field.optional {
            quote! {
                if let Some(__field) = self.#ident {
                    __object.set_named(#js_name, ::wae::ToJs::to_js(__field, __env)?)?;
                }
            }
        }
        else {
            quote! {
                __object.set_named(#js_name, ::wae::ToJs::to_js(self.#ident, __env)?)?;
            }
        }
    });

    let ts_fields = collected.iter().map(|field| {
        let js_name = &field.js_name;
        let ty = option_inner(&field.ty).unwrap_or(&field.ty);
        let key = if field.optional { format!("{js_name}?") } else { js_name.clone() };
        quote! {
            __fields.push(format!("    {}: {}", #key, <#ty as ::wae::TsType>::ts_type()));
        }
    });
    let declarations = collected.iter().map(|field| {
        let ty = &field.ty;
        quote! { <#ty as ::wae::TsType>::declarations(__out); }
    });
    let ts_name = name.to_string();

    Ok(quote! {
        #cleaned

        impl ::wae::FromJs for #name {
            fn from_js(__value: ::wae::JsValue) -> ::wae::Result<Self> {
                Ok(Self {
                    #(#from_fields)*
                })
            }
        }

        impl ::wae::ToJs for #name {
            fn to_js(self, __env: ::wae::Env) -> ::wae::Result<::wae::JsValue> {
                let __object = __env.object()?;
                #(#to_fields)*
                Ok(__object)
            }
        }

        impl ::wae::TsType for #name {
            fn ts_type() -> String {
                #ts_name.to_string()
            }

            fn declarations(__out: &mut Vec<::wae::TypeDecl>) {
                let mut __fields: Vec<String> = Vec::new();
                #(#ts_fields)*
                let __body = if __fields.is_empty() {
                    "{}".to_string()
                }
                else {
                    format!("{{\n{}\n}}", __fields.join("\n"))
                };
                __out.push(::wae::TypeDecl {
                    name: #ts_name.to_string(),
                    text: format!("export interface {} {}", #ts_name, __body),
                });
                #(#declarations)*
            }
        }
    })
}
