//! WAE 过程宏：N-API 函数导出与模块注册。
//!
//! 生成代码一律通过门面 crate `::wae` 解析类型与辅助函数。

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, ExprLit, FnArg, ItemFn, Lit, Meta, MetaNameValue, Token, parse::Parser, parse_macro_input, punctuated::Punctuated};

/// 将 `fn(CallContext) -> Result<JsValue>` 包成 Node-API `extern "C"` 回调。
///
/// 可选参数：`js_name = "..."`（默认用 Rust 函数标识符原文）。
///
/// 展开后额外生成：
/// - `wae_napi_export_<name>`：C ABI 回调
/// - `WAE_NAPI_EXPORT_<name>`：`(js_name, 回调)` 常量，供 [`register_module`] 使用
#[proc_macro_attribute]
pub fn function(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match parse_attribute_args(attr) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    let input = parse_macro_input!(item as ItemFn);
    match expand_function(args, input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// 生成 `napi_register_module_v1`，参数为已用 [`function`] 标注的 Rust 函数名列表。
///
/// ```ignore
/// wae::register_module![scan, version, open_gui];
/// ```
#[proc_macro]
pub fn register_module(input: TokenStream) -> TokenStream {
    let parser = Punctuated::<syn::Ident, Token![,]>::parse_terminated;
    let idents = match parser.parse(input) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    if idents.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "register_module! 至少需要一个导出函数").to_compile_error().into();
    }

    let entries = idents.iter().map(|id| {
        let const_name = format_ident!("WAE_NAPI_EXPORT_{}", id.to_string().to_ascii_uppercase());
        quote! { #const_name }
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
            match ::wae::register_exports(
                env,
                exports,
                &[
                    #(#entries),*
                ],
            ) {
                Ok(v) => v,
                Err(err) => {
                    ::wae::Env::from_raw(env).throw_error(err.message());
                    exports
                }
            }
        }
    }
    .into()
}

struct FunctionArgs {
    js_name: Option<String>,
}

fn parse_attribute_args(attr: TokenStream) -> syn::Result<FunctionArgs> {
    if attr.is_empty() {
        return Ok(FunctionArgs { js_name: None });
    }
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse(attr)?;
    let mut js_name = None;
    for meta in metas {
        match meta {
            Meta::NameValue(MetaNameValue { path, value, .. }) if path.is_ident("js_name") => {
                js_name = Some(expr_to_string(&value)?);
            }
            other => {
                return Err(syn::Error::new_spanned(other, "仅支持 js_name = \"...\""));
            }
        }
    }
    Ok(FunctionArgs { js_name })
}

fn expr_to_string(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => Ok(s.value()),
        other => Err(syn::Error::new_spanned(other, "js_name 必须是字符串字面量")),
    }
}

fn expand_function(args: FunctionArgs, mut input: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    if input.sig.inputs.len() != 1 {
        return Err(syn::Error::new_spanned(&input.sig.inputs, "#[function] 只接受一个 CallContext 参数"));
    }
    if !matches!(input.sig.inputs.first(), Some(FnArg::Typed(_))) {
        return Err(syn::Error::new_spanned(&input.sig, "#[function] 需要签名：fn(CallContext) -> Result<JsValue>"));
    }

    let fn_ident = input.sig.ident.clone();
    let js_name = args.js_name.unwrap_or_else(|| fn_ident.to_string());
    let export_fn = format_ident!("wae_napi_export_{}", fn_ident);
    let export_const = format_ident!("WAE_NAPI_EXPORT_{}", fn_ident.to_string().to_ascii_uppercase());
    let vis = input.vis.clone();
    input.attrs.retain(|a| !a.path().is_ident("function"));

    Ok(quote! {
        #input

        #[doc(hidden)]
        #vis unsafe extern "C" fn #export_fn(
            env: ::wae::napi_env,
            info: ::wae::napi_callback_info,
        ) -> ::wae::napi_value {
            unsafe { ::wae::catch_callback(env, info, #fn_ident) }
        }

        #[doc(hidden)]
        #vis const #export_const: (
            &'static str,
            unsafe extern "C" fn(::wae::napi_env, ::wae::napi_callback_info) -> ::wae::napi_value,
        ) = (#js_name, #export_fn);
    })
}
