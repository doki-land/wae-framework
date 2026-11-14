//! WAE Rust **门面库**：统一导出子 crate 与过程宏。
//!
//! 下游 addon 通常只依赖本 crate：
//!
//! ```ignore
//! use wae::{function, register_module, CallContext, JsValue, Result};
//!
//! #[function]
//! fn version(ctx: CallContext) -> Result<JsValue> {
//!     ctx.env().string(env!("CARGO_PKG_VERSION"))
//! }
//!
//! register_module![version];
//! ```
//!
//! WebView 放在子模块，避免与 N-API 的 `Result` 撞名：`wae::webview::*`。
//!
//! 元 CLI 在 TypeScript（`@wae/wae`）；本 crate **不是** 可执行 bin。

#![deny(missing_docs)]

pub use wae_napi::*;

pub use wae_macros::{function, object, register_module};

/// `wae::webview`：按编译目标选择 WebView 后端（feature `webview`，默认开启）。
///
/// Windows → `wae-webview-win32`，macOS → `wae-webview-macos`，
/// Linux → `wae-webview-linux`。其它目标在编译期显式失败。
///
/// # 下游约束（强制）
///
/// 本模块在三个桌面平台暴露**同一组符号与签名**。下游应用（含 `napi` 层与
/// `@wae/*` 消费者）**不得**写 `#[cfg(windows)]` / `cfg!(windows)` 之类的平台判断，
/// 也不得用字符串表达「仅 Windows 支持」。平台差异只在两处出现：后端 crate 内部，
/// 以及本模块的后端选择层。平台缺少能力时由门面返回
/// [`WebViewError::Unsupported`]（携带 [`Capability`] 与 [`Platform`]），下游按字段判别。
///
/// ```ignore
/// #[function(js_name = "openGui")]
/// fn open_gui(ctx: CallContext) -> Result<JsValue> {
///     // 一行转发即可在 Windows / macOS / Linux 编译
///     let host = wae::webview::open(options, Box::new(handler))?;
///     // ...
/// }
/// ```
#[cfg(feature = "webview")]
pub mod webview {
    pub use wae_webview::{Capability, IpcHandler, IpcOutcome, IpcSink, OpenOptions, Platform, Result, WebViewError};

    /// 当前编译目标对应的平台，供结构化错误与诊断使用。
    pub fn current_platform() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        }
        else if cfg!(target_os = "macos") {
            Platform::Macos
        }
        else {
            Platform::Linux
        }
    }

    /// 打开 WebView 宿主；与 `WebViewHost::open` 等价，供应用侧一行转发。
    pub fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<WebViewHost> {
        WebViewHost::open(options, handler)
    }

    /// 当前目标的 WebView 宿主（Windows：Win32 + WebView2）。
    #[cfg(target_os = "windows")]
    pub type WebViewHost = wae_webview::WebViewHost<wae_webview_win32::Win32Backend>;

    /// 当前目标的 WebView 宿主（macOS：WKWebView）。
    #[cfg(target_os = "macos")]
    pub type WebViewHost = wae_webview::WebViewHost<wae_webview_macos::MacosBackend>;

    /// 当前目标的 WebView 宿主（Linux：WebKitGTK）。
    #[cfg(target_os = "linux")]
    pub type WebViewHost = wae_webview::WebViewHost<wae_webview_linux::LinuxBackend>;

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    compile_error!("`wae::webview` 只支持 Windows / macOS / Linux；其它目标没有 WebView 后端");
}

/// `wae::shell`：用系统关联程序启动文件（feature `shell`，默认开启）。
///
/// 三个桌面平台同一签名 `open_path(&Path) -> Result<(), ShellError>`；
/// 失败按 [`ShellError`] 字段判别，下游不得写平台 `cfg`、不得匹配错误字符串。
/// 调用方应只传入自己产出的真实文件路径（如 OpenZipper 的临时预览文件）。
///
/// ```ignore
/// wae::shell::open_path(std::path::Path::new(&temp_file))?;
/// ```
#[cfg(feature = "shell")]
pub mod shell {
    pub use wae_shell::{ShellError, open_path};
}

/// `wae::audit`：审计记录设施与 napi 复用入口（feature `audit`，默认开启）。
///
/// 所有调用都必须可归因到具体 `user` 或 `agent`。应用只补领域字段（能力名、参数、
/// 目标数量与摘要、错误码、模式），公共字段（时间戳、身份、应用版本、主机、会话与
/// 耗时）由 [`Auditor`] 补齐。
///
/// `actor` 只记录身份，`role` / 权限判定属于独立授权模型。授权失败用
/// [`Call::reject`] 记为 `rejected`，不得改写 actor 或把 role 写进 actor。
///
/// ```ignore
/// let auditor = wae::audit::Auditor::from_env("open-cleaner", env!("CARGO_PKG_VERSION"));
///
/// #[function(js_name = "scan")]
/// fn scan(ctx: CallContext) -> Result<JsValue> {
///     wae::audit::napi_call(&auditor, "scan", wae::audit::Mode::Execute)
///         .targets(1, "scan roots")
///         .run(|| do_scan(ctx))
/// }
///
/// #[function(js_name = "auditQuery")]
/// fn audit_query(ctx: CallContext) -> Result<JsValue> {
///     wae::audit::query_from_napi(&ctx, &auditor)
/// }
/// ```
#[cfg(feature = "audit")]
pub mod audit {
    pub use wae_audit::*;

    use crate::{CallContext, Error, JsValue, Result, optional_string_prop, sys};

    /// 一次 napi 调用的审计构造；[`NapiCall::run`] 负责记录成功或失败。
    pub struct NapiCall<'a> {
        call: Option<wae_audit::Call<'a>>,
        error_code: String,
    }

    impl NapiCall<'_> {
        /// 设置 JSON 文本参数，脱敏由设施完成。
        pub fn params_json(mut self, text: Option<&str>) -> Self {
            if let Some(text) = text
                && let Some(call) = self.call.take()
            {
                self.call = Some(call.params_json(text));
            }
            self
        }

        /// 设置目标数量与摘要。
        pub fn targets(mut self, count: u64, summary: impl Into<String>) -> Self {
            if let Some(call) = self.call.take() {
                self.call = Some(call.targets(count, summary));
            }
            self
        }

        /// 覆盖失败时的稳定错误码，默认 `wae.audit::error`。
        pub fn error_code(mut self, code: impl Into<String>) -> Self {
            self.error_code = code.into();
            self
        }

        /// 执行领域调用并记录结果。
        ///
        /// [`Error::refused`] 标记的错误记为 `rejected` 并写入其稳定 code，
        /// 其余记为 `failure` 并回落 [`NapiCall::error_code`] 的默认值。
        pub fn run<F>(mut self, run: F) -> Result<JsValue>
        where
            F: FnOnce() -> Result<JsValue>,
        {
            let Some(call) = self.call.take()
            else {
                return Err(Error::new("wae.audit::napi_call_finished"));
            };
            match run() {
                Ok(value) => {
                    call.succeed().map_err(|err| Error::new(err.to_string()))?;
                    Ok(value)
                }
                Err(err) => {
                    let code = err.code().unwrap_or(self.error_code.as_str());
                    let recorded = if err.is_refusal() { call.reject(code) } else { call.fail(code) };
                    recorded.map_err(|audit| Error::new(audit.to_string()))?;
                    Err(err)
                }
            }
        }
    }

    /// 从审计器构造 napi 调用记录入口。
    pub fn napi_call<'a>(auditor: &'a wae_audit::Auditor, capability: impl Into<String>, mode: wae_audit::Mode) -> NapiCall<'a> {
        NapiCall { call: Some(auditor.call(capability, mode)), error_code: "wae.audit::error".into() }
    }

    /// 从 JS 过滤对象查询审计记录，返回 JSON 数组文本。
    ///
    /// 过滤对象字段：`kind`、`id`、`fromMs`、`toMs`、`capability`、`limit`。
    pub fn query_from_napi(ctx: &CallContext, auditor: &wae_audit::Auditor) -> Result<JsValue> {
        let filter = filter_from_js(ctx.args().first().copied())?;
        let json = auditor.query_json(&filter).map_err(|err| Error::new(err.to_string()))?;
        ctx.env().string(&json)
    }

    fn filter_from_js(arg: Option<JsValue>) -> Result<wae_audit::Query> {
        let mut filter = wae_audit::Query::default();
        let Some(arg) = arg
        else {
            return Ok(filter);
        };
        if arg.is_undefined()? {
            return Ok(filter);
        }
        filter.actor_kind = match optional_string_prop(arg, "kind")? {
            Some(value) if value.eq_ignore_ascii_case("agent") => Some(wae_audit::ActorKind::Agent),
            Some(_) => Some(wae_audit::ActorKind::User),
            None => None,
        };
        filter.actor_id = optional_string_prop(arg, "id")?;
        filter.capability = optional_string_prop(arg, "capability")?;
        filter.from_ms = optional_number_prop(arg, "fromMs")?;
        filter.to_ms = optional_number_prop(arg, "toMs")?;
        filter.limit = optional_number_prop(arg, "limit")?.map(|value| value as usize);
        Ok(filter)
    }

    fn optional_number_prop(obj: JsValue, name: &str) -> Result<Option<u64>> {
        let value = obj.get_named(name)?;
        if value.is_undefined()? || value.type_of()? != sys::napi_number {
            return Ok(None);
        }
        Ok(Some(value.as_i64()?.max(0) as u64))
    }
}

/// `wae::plugins`：声明式插件宿主（feature `plugins`，默认开启）。
///
/// 五个 Open 应用共用同一份加载器：清单解析、作用域收窄、候选归因与错误码只在本
/// 处实现。插件只提交候选与处置请求，判定与执行永远在核心。
///
/// ```ignore
/// use wae::plugins::{CandidateSet, PackContext, load_pack};
///
/// let context = PackContext::new(&["open-cleaner"], &["dev-artifacts"], &[]);
/// let pack = load_pack(std::path::Path::new("./plugins/cleaner-rules"), &context)?;
/// ```
#[cfg(feature = "plugins")]
pub mod plugins {
    pub use wae_plugins::*;
}
