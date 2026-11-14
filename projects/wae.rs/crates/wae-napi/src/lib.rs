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
pub use meta::{ClassMeta, FunctionMeta, MethodKind, MethodMeta, ParamMeta, TsType, TypeDecl, metadata_json, metadata_json_all};

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
    detail: Option<serde_json::Value>,
}

impl Error {
    /// 由说明文本构造错误。
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), code: None, refused: false, detail: None }
    }

    /// 调用方输入非法：参数缺失、类型不符、取值越界。
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(message).code("invalid_input")
    }

    /// 找不到目标资源（文件、预设、条目）。
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(message).code("not_found")
    }

    /// 内部失败：I/O、序列化、后端错误。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(message).code("internal")
    }

    /// 借用错误说明。
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 设置稳定 code；JS 侧从 `error.code` 读取。
    pub fn code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// 标记为能力或授权拒绝，审计记为 `rejected`。
    pub fn refused(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self.refused = true;
        self
    }

    /// 附加结构化细节；JS 侧从 `error.detail` 读取。
    pub fn detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    /// 稳定 code，缺失时由审计入口回落到默认值。
    pub fn code_name(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// 结构化细节。
    pub fn detail_value(&self) -> Option<&serde_json::Value> {
        self.detail.as_ref()
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

    /// 把 [`Error`] 构造成 JS `Error` 实例，带 `code` 与可选 `detail` 属性。
    ///
    /// 这条路是错误契约的唯一出口：产品读 `error.code` / `error.detail`，
    /// 不再解析文本前缀。
    pub fn error(self, error: &Error) -> Result<JsValue> {
        let message = self.string(error.message())?;
        let code = match error.code_name() {
            Some(code) => self.string(code)?.raw(),
            None => ptr::null_mut(),
        };
        let mut out = ptr::null_mut();
        check(unsafe { napi_create_error(self.0, code, message.raw(), &mut out) })?;
        let value = JsValue { env: self, value: out };
        if let Some(detail) = error.detail_value() {
            value.set_named("detail", json_to_js(self, detail)?)?;
        }
        Ok(value)
    }

    /// 抛出任意 JS 值（通常是 [`Env::error`] 构造的错误对象）。
    pub fn throw(self, value: JsValue) -> Result<()> {
        check(unsafe { napi_throw(self.0, value.raw()) })
    }

    /// 创建 Promise 与其 deferred 句柄。
    ///
    /// 任务结束时用 [`Env::resolve`] 或 [`Env::reject`] 结算，二者只能在主线程调用。
    pub fn promise(self) -> Result<(JsValue, napi_deferred)> {
        let mut deferred = ptr::null_mut();
        let mut promise = ptr::null_mut();
        check(unsafe { napi_create_promise(self.0, &mut deferred, &mut promise) })?;
        Ok((JsValue { env: self, value: promise }, deferred))
    }

    /// 用值结算 Promise。
    pub fn resolve(self, deferred: napi_deferred, value: JsValue) -> Result<()> {
        check(unsafe { napi_resolve_deferred(self.0, deferred, value.raw()) })
    }

    /// 用错误对象拒绝 Promise。
    pub fn reject(self, deferred: napi_deferred, error: JsValue) -> Result<()> {
        check(unsafe { napi_reject_deferred(self.0, deferred, error.raw()) })
    }

    /// 在 **Node 托管线程池** 上排一个任务。
    ///
    /// `execute` 在工作线程运行，不得触碰 `Env` / `JsValue`；`complete` 在主线程运行，
    /// 负责转换返回值、结算 Promise 并删除任务。
    pub fn async_work(
        self,
        name: &str,
        data: *mut std::ffi::c_void,
        execute: napi_async_execute_callback,
        complete: napi_async_complete_callback,
    ) -> Result<napi_async_work> {
        let resource_name = self.string(name)?;
        let mut work = ptr::null_mut();
        check(unsafe { napi_create_async_work(self.0, ptr::null_mut(), resource_name.raw(), execute, complete, data, &mut work) })?;
        Ok(work)
    }

    /// 把任务交给线程池。
    pub fn queue_work(self, work: napi_async_work) -> Result<()> {
        check(unsafe { napi_queue_async_work(self.0, work) })
    }

    /// 删除任务句柄；在 complete 回调里调用。
    pub fn delete_work(self, work: napi_async_work) {
        unsafe {
            let _ = napi_delete_async_work(self.0, work);
        }
    }

    /// 定义一个 JS 类；`constructor` 必须用 [`Env::wrap`] 挂上实例。
    pub fn define_class(self, name: &str, constructor: napi_callback, properties: &[napi_property_descriptor]) -> Result<JsValue> {
        let c_name = CString::new(name).map_err(|error| Error::new(error.to_string()))?;
        let mut out = ptr::null_mut();
        check(unsafe {
            napi_define_class(
                self.0,
                c_name.as_ptr(),
                name.len(),
                constructor,
                ptr::null_mut(),
                properties.as_ptr(),
                properties.len(),
                &mut out,
            )
        })?;
        Ok(JsValue { env: self, value: out })
    }

    /// 把 `Box<T>` 实例挂到 JS 对象上，`finalize` 负责回收。
    pub fn wrap(self, object: JsValue, native: *mut std::ffi::c_void, finalize: napi_finalize) -> Result<()> {
        check(unsafe { napi_wrap(self.0, object.raw(), native, finalize, ptr::null_mut(), ptr::null_mut()) })
    }

    /// 取回 [`Env::wrap`] 挂上的实例指针。
    pub fn unwrap(self, object: JsValue) -> Result<*mut std::ffi::c_void> {
        let mut out = ptr::null_mut();
        check(unsafe { napi_unwrap(self.0, object.raw(), &mut out) })?;
        if out.is_null() {
            return Err(Error::new("receiver is not an instance of this class"));
        }
        Ok(out)
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
    this: Option<JsValue>,
    new_target: bool,
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
        let mut this = ptr::null_mut();
        check(unsafe { napi_get_cb_info(env, info, &mut argc, argv.as_mut_ptr(), &mut this, ptr::null_mut()) })?;
        let mut args = argv[..argc].to_vec();
        if argc == argv.len() {
            let mut wide = 64usize;
            let mut buffer = [ptr::null_mut(); 64];
            check(unsafe { napi_get_cb_info(env, info, &mut wide, buffer.as_mut_ptr(), &mut this, ptr::null_mut()) })?;
            if wide == buffer.len() {
                return Err(Error::new("too many arguments: the call received 64 or more"));
            }
            args = buffer[..wide].to_vec();
        }
        let env = Env::from_raw(env);
        let args = args.into_iter().map(|value| JsValue::from_raw(env, value)).collect();
        let this = if this.is_null() { None } else { Some(JsValue::from_raw(env, this)) };
        let mut new_target = ptr::null_mut();
        check(unsafe { napi_get_new_target(env.raw(), info, &mut new_target) })?;
        Ok(Self { env, args, this, new_target: !new_target.is_null() })
    }

    /// 回调环境。
    pub fn env(&self) -> Env {
        self.env
    }

    /// 位置参数列表。
    pub fn args(&self) -> &[JsValue] {
        &self.args
    }

    /// 方法调用时的 `this`；普通函数调用没有接收者。
    pub fn this(&self) -> Result<JsValue> {
        self.this.ok_or_else(|| Error::new("this call has no receiver"))
    }

    /// 本次调用是否通过 `new` 发起（构造器判定）。
    pub fn is_constructor(&self) -> bool {
        self.new_target
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
    classes: &[(&str, unsafe extern "C" fn(napi_env, napi_value) -> napi_status)],
    metas: &[fn() -> FunctionMeta],
    class_metas: &[fn() -> ClassMeta],
) -> Result<napi_value> {
    let raw = register_exports(env, exports, methods)?;
    let env = Env::from_raw(env);
    let target = JsValue::from_raw(env, raw);
    for (name, register) in classes {
        let status = unsafe { register(env.raw(), raw) };
        if status != 0 {
            return Err(Error::new(format!("failed to register class `{name}`")));
        }
    }
    let types = metadata_json_all(metas, class_metas);
    target.set_named("__waeTypes", env.string(&types)?)?;
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
        Ok(value) => value.raw(),
        Err(error) => {
            // 结构化错误优先：JS 侧读 `error.code` / `error.detail`。
            match env_wrap.error(&error).and_then(|value| env_wrap.throw(value)) {
                Ok(()) => ptr::null_mut(),
                Err(_) => {
                    env_wrap.throw_error(error.message());
                    ptr::null_mut()
                }
            }
        }
    }
}

/// 线程池任务句柄。
pub use sys::napi_async_work;
/// 原生回调签名。
pub use sys::napi_callback;
/// Node-API 回调信息句柄。
pub use sys::napi_callback_info;
/// Promise 的延迟结算句柄。
pub use sys::napi_deferred;
/// Node-API 环境句柄。
pub use sys::napi_env;
/// 实例回收回调。
pub use sys::napi_finalize;
/// 类方法的属性描述符。
pub use sys::napi_property_descriptor;
/// Node-API 状态码。
pub use sys::napi_status;
/// Node-API 值句柄。
pub use sys::napi_value;
