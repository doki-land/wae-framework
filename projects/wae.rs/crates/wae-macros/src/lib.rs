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
    Data, DeriveInput, Expr, ExprLit, Fields, FnArg, ItemFn, Lit, Meta, MetaNameValue, Pat, ReturnType, Token, Type, TypePath, parse::Parser,
    parse_macro_input, punctuated::Punctuated,
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
    let parser = Punctuated::<syn::Ident, Token![,]>::parse_terminated;
    let idents = match parser.parse(input) {
        Ok(value) => value,
        Err(error) => return error.to_compile_error().into(),
    };
    if idents.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "register_module! 至少需要一个导出函数").to_compile_error().into();
    }

    let entries = idents.iter().map(|id| {
        let const_name = format_ident!("WAE_NAPI_EXPORT_{}", id.to_string().to_ascii_uppercase());
        quote! { #const_name }
    });
    let metas = idents.iter().map(|id| {
        let meta_name = format_ident!("wae_meta_{}", id);
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
                    #(#metas),*
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

struct FunctionArgs {
    js_name: Option<String>,
}

fn parse_function_args(attr: TokenStream) -> syn::Result<FunctionArgs> {
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

    let body = if untyped && data.is_empty() {
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

    Ok(quote! {
        #output

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
                declarations: #decl_fn,
            }
        }
    })
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
