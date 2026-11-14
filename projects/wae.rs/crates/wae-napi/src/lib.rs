//! 自研薄 Node-API 绑定——不依赖 `napi` / `napi-derive` / `napi-build`。
//!
//! 链接模型对齐 napi-rs 的 **dyn-symbols**：在 addon 加载时用 `libloading`
//! 从宿主进程解析 `napi_*`，而不是在链接期伪造 `node.exe` import lib。

#![deny(missing_docs)]
#![allow(non_upper_case_globals)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

/// 原始 Node-API C ABI 与运行时符号表。
///
/// 该模块以 FFI 声明为主，逐项补文档收益很低；对外请优先使用本 crate 的安全封装。
#[allow(missing_docs)]
pub mod sys;

/// 从宿主进程解析 Node-API 符号（模块初始化时调用一次即可）。
pub use sys::setup;

mod convert;
mod meta;

pub use convert::{FromJs, Json, ToJs, js_to_json, json_to_js};
pub use meta::{FunctionMeta, ParamMeta, TsType, TypeDecl, metadata_json};

use std::{
    ffi::{CStr, CString},
    ptr,
};

use sys::*;

/// 薄 N-API 辅助层抛出的错误。
///
/// 除说明文本外，错误可以携带审计使用的稳定 code，并区分「执行失败」与
/// 「能力/授权拒绝」。审计入口据此写入 `failure` 或 `rejected`，不靠文本猜测。
#[derive(Debug)]
pub struct Error {
    message: String,
    code: Option<String>,
    refused: bool,
}

impl Error {
    /// 由说明文本构造错误。
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), code: None, refused: false }
    }

    /// 借用错误说明。
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 标记为能力或授权拒绝，审计记为 `rejected`。
    pub fn refused(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self.refused = true;
        self
    }

    /// 稳定 code，缺失时由审计入口回落到默认值。
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// 是否属于能力或授权拒绝。
    pub fn is_refusal(&self) -> bool {
        self.refused
    }

    /// 给错误加一层上下文前缀，保留稳定 code 与拒绝标记。
    ///
    /// 参数转换用它与调用点组合出 `argument 0 (options): expected object, got number`。
    pub fn context(mut self, context: impl std::fmt::Display) -> Self {
        self.message = format!("{context}: {}", self.message);
        self
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// 薄 N-API 辅助层的结果别名。
pub type Result<T> = std::result::Result<T, Error>;

/// Node-API 环境句柄封装。
#[derive(Clone, Copy)]
pub struct Env(napi_env);

impl Env {
    /// 包装原始 `napi_env`。
    pub fn from_raw(env: napi_env) -> Self {
        Self(env)
    }

    /// 取出原始 `napi_env`。
    pub fn raw(self) -> napi_env {
        self.0
    }

    /// 创建 JS `undefined`。
    pub fn undefined(self) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_get_undefined(self.0, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建 JS `null`。
    pub fn null(self) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_get_null(self.0, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建 JS 布尔值。
    pub fn boolean(self, value: bool) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_get_boolean(self.0, value, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建 JS UTF-8 字符串。
    pub fn string(self, value: &str) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_string_utf8(self.0, value.as_ptr() as *const _, value.len(), &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建 JS int64 数字。
    pub fn int64(self, value: i64) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_int64(self.0, value, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建 JS double 数字。
    pub fn double(self, value: f64) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_double(self.0, value, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建空的 JS 对象。
    pub fn object(self) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_object(self.0, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建指定长度的 JS 数组。
    pub fn array_with_length(self, len: usize) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_array_with_length(self.0, len, &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 创建绑定到原生回调的 JS 函数。
    pub fn function(self, name: &str, cb: unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value) -> Result<JsValue> {
        let c_name = CString::new(name).map_err(|e| Error::new(e.to_string()))?;
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_function(self.0, c_name.as_ptr(), name.len(), Some(cb), ptr::null_mut(), &mut out) })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 抛出带说明文本的 JS `Error`。
    pub fn throw_error(self, message: &str) {
        if let Ok(c) = CString::new(message) {
            unsafe {
                let _ = napi_throw_error(self.0, ptr::null(), c.as_ptr());
            }
        }
    }
}

/// Node-API 值句柄封装。
#[derive(Clone, Copy)]
pub struct JsValue {
    env: Env,
    value: napi_value,
}

impl JsValue {
    /// 包装原始 `napi_value`。
    pub fn from_raw(env: Env, value: napi_value) -> Self {
        Self { env, value }
    }

    /// 取出原始 `napi_value`。
    pub fn raw(self) -> napi_value {
        self.value
    }

    /// 该值所属的环境。
    pub fn env(self) -> Env {
        self.env
    }

    /// 在对象上设置命名属性。
    pub fn set_named(self, name: &str, value: JsValue) -> Result<()> {
        let c_name = CString::new(name).map_err(|e| Error::new(e.to_string()))?;
        check(unsafe { napi_set_named_property(self.env.raw(), self.value, c_name.as_ptr(), value.raw()) })
    }

    /// 读取对象上的命名属性。
    pub fn get_named(self, name: &str) -> Result<JsValue> {
        let c_name = CString::new(name).map_err(|e| Error::new(e.to_string()))?;
        let mut out = ptr::null_mut();
        check(unsafe { napi_get_named_property(self.env.raw(), self.value, c_name.as_ptr(), &mut out) })?;
        Ok(JsValue { env: self.env, value: out })
    }

    /// 设置数组/对象的下标元素。
    pub fn set_element(self, index: u32, value: JsValue) -> Result<()> {
        check(unsafe { napi_set_element(self.env.raw(), self.value, index, value.raw()) })
    }

    /// 将值转为 Rust `String`。
    pub fn as_string(self) -> Result<String> {
        let mut len = 0usize;
        check(unsafe { napi_get_value_string_utf8(self.env.raw(), self.value, ptr::null_mut(), 0, &mut len) })?;
        let mut buf = vec![0u8; len + 1];
        let mut written = 0usize;
        check(unsafe { napi_get_value_string_utf8(self.env.raw(), self.value, buf.as_mut_ptr() as *mut _, buf.len(), &mut written) })?;
        buf.truncate(written);
        String::from_utf8(buf).map_err(|e| Error::new(e.to_string()))
    }

    /// 将值转为 `bool`。
    pub fn as_bool(self) -> Result<bool> {
        let mut out = false;
        check(unsafe { napi_get_value_bool(self.env.raw(), self.value, &mut out) })?;
        Ok(out)
    }

    /// 将值转为 `i64`。
    pub fn as_i64(self) -> Result<i64> {
        let mut out = 0i64;
        check(unsafe { napi_get_value_int64(self.env.raw(), self.value, &mut out) })?;
        Ok(out)
    }

    /// 将值转为 `f64`。
    pub fn as_f64(self) -> Result<f64> {
        let mut out = 0f64;
        check(unsafe { napi_get_value_double(self.env.raw(), self.value, &mut out) })?;
        Ok(out)
    }

    /// 返回该值的 Node-API 类型标签。
    pub fn type_of(self) -> Result<i32> {
        let mut ty = 0i32;
        check(unsafe { napi_typeof(self.env.raw(), self.value, &mut ty) })?;
        Ok(ty)
    }

    /// 是否为 JS `undefined`。
    pub fn is_undefined(self) -> Result<bool> {
        Ok(self.type_of()? == napi_undefined)
    }

    /// 是否为 JS 数组。
    pub fn is_array(self) -> Result<bool> {
        let mut result = false;
        check(unsafe { napi_is_array(self.env.raw(), self.value, &mut result) })?;
        Ok(result)
    }

    /// 读取数组下标元素。
    pub fn element(self, index: u32) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_get_element(self.env.raw(), self.value, index, &mut out) })?;
        Ok(JsValue { env: self.env, value: out })
    }

    /// 读取对象自有属性名，返回字符串数组。
    pub fn property_names(self) -> Result<JsValue> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_get_property_names(self.env.raw(), self.value, &mut out) })?;
        Ok(JsValue { env: self.env, value: out })
    }

    /// JS 侧类型名，只用于错误文本；无法判别时返回 `unknown`。
    pub fn describe(self) -> String {
        let Ok(kind) = self.type_of()
        else {
            return "unknown".to_string();
        };
        match kind {
            napi_undefined => "undefined",
            sys::napi_null => "null",
            sys::napi_boolean => "boolean",
            sys::napi_number => "number",
            sys::napi_string => "string",
            sys::napi_symbol => "symbol",
            sys::napi_function => "function",
            sys::napi_external => "external",
            sys::napi_bigint => "bigint",
            sys::napi_object if self.is_array().unwrap_or(false) => "array",
            sys::napi_object => "object",
            _ => "unknown",
        }
        .to_string()
    }
}

/// 原生 JS 回调的参数与环境。
pub struct CallContext {
    env: Env,
    args: Vec<JsValue>,
}

impl CallContext {
    /// 由原始 Node-API 回调信息构造调用上下文。
    ///
    /// 参数数量按实际传入值决定：先按 8 个取，若恰好填满则扩容重取，仍填满则显式失败，
    /// 不再静默截断第 17 个及以后的参数。
    ///
    /// # Safety
    /// 在回调存活期间，`env` 与 `info` 必须有效。
    pub unsafe fn from_callback_info(env: napi_env, info: napi_callback_info) -> Result<Self> {
        let mut argc = 8usize;
        let mut argv = [ptr::null_mut(); 8];
        check(unsafe { napi_get_cb_info(env, info, &mut argc, argv.as_mut_ptr(), ptr::null_mut(), ptr::null_mut()) })?;
        let mut args = argv[..argc].to_vec();
        if argc == argv.len() {
            let mut wide = 64usize;
            let mut buffer = [ptr::null_mut(); 64];
            check(unsafe { napi_get_cb_info(env, info, &mut wide, buffer.as_mut_ptr(), ptr::null_mut(), ptr::null_mut()) })?;
            if wide == buffer.len() {
                return Err(Error::new("too many arguments: the call received 64 or more"));
            }
            args = buffer[..wide].to_vec();
        }
        let env = Env::from_raw(env);
        let args = args.into_iter().map(|value| JsValue::from_raw(env, value)).collect();
        Ok(Self { env, args })
    }

    /// 回调环境。
    pub fn env(&self) -> Env {
        self.env
    }

    /// 位置参数列表。
    pub fn args(&self) -> &[JsValue] {
        &self.args
    }

    /// 按索引取位置参数。
    pub fn arg(&self, index: usize) -> Result<JsValue> {
        self.args.get(index).copied().ok_or_else(|| Error::new(format!("缺少参数 {index}")))
    }
}

fn check(status: napi_status) -> Result<()> {
    if status == napi_ok { Ok(()) } else { Err(Error::new(format!("napi status {status}"))) }
}

/// 注册 Node addon 导出。通常在 `napi_register_module_v1` 中调用。
pub fn register_exports(
    env: napi_env,
    exports: napi_value,
    methods: &[(&str, unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value)],
) -> Result<napi_value> {
    let env = Env::from_raw(env);
    let exports = JsValue::from_raw(env, exports);
    for (name, cb) in methods {
        let func = env.function(name, *cb)?;
        exports.set_named(name, func)?;
    }
    Ok(exports.raw())
}

/// 注册 Node addon 导出，同时挂上 `__waeTypes` 类型元数据。
///
/// `__waeTypes` 是下划线隐藏面：`@wae/wae` 的 `types` 构建步与 `wae doctor` 读它，
/// 不属于公开 API 文档。
pub fn register_exports_with_types(
    env: napi_env,
    exports: napi_value,
    methods: &[(&str, unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value)],
    metas: &[fn() -> FunctionMeta],
) -> Result<napi_value> {
    let raw = register_exports(env, exports, methods)?;
    let env = Env::from_raw(env);
    let types = metadata_json(metas);
    JsValue::from_raw(env, raw).set_named("__waeTypes", env.string(&types)?)?;
    Ok(raw)
}

/// 从 JS 对象参数中读取可选字符串属性。
pub fn optional_string_prop(obj: JsValue, name: &str) -> Result<Option<String>> {
    let v = obj.get_named(name)?;
    if v.is_undefined()? || v.type_of()? == napi_null {
        return Ok(None);
    }
    if v.type_of()? != napi_string {
        return Ok(None);
    }
    Ok(Some(v.as_string()?))
}

/// 将 `CStr` 转为 `&str`（非法 UTF-8 时返回空串）。
#[doc(hidden)]
pub fn cstr(s: &CStr) -> &str {
    s.to_str().unwrap_or("")
}

/// 运行类型化回调，并把 Rust 错误转换为抛出的 JS 异常。
///
/// # Safety
/// `env` 与 `info` 必须是当前回调有效的 Node-API 句柄。
pub unsafe fn catch_callback<F>(env: napi_env, info: napi_callback_info, f: F) -> napi_value
where
    F: FnOnce(CallContext) -> Result<JsValue>,
{
    let env_wrap = Env::from_raw(env);
    match (|| {
        let ctx = unsafe { CallContext::from_callback_info(env, info)? };
        f(ctx)
    })() {
        Ok(v) => v.raw(),
        Err(err) => {
            env_wrap.throw_error(err.message());
            ptr::null_mut()
        }
    }
}

/// Node-API 回调信息句柄。
pub use sys::napi_callback_info;
/// Node-API 环境句柄。
pub use sys::napi_env;
/// Node-API 值句柄。
pub use sys::napi_value;
