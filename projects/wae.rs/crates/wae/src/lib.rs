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
//! WebView / dist 放在子模块，避免与 N-API 的 `Result` 撞名：
//! `wae::webview::*`、`wae::dist::*`。
//!
//! 元 CLI 在 TypeScript（`@wae/wae`）；本 crate **不是** 可执行 bin。

#![deny(missing_docs)]

pub use wae_napi::*;

pub use wae_macros::{function, register_module};

/// `wae-dist` 暂存 API（feature `dist`，默认开启）。
#[cfg(feature = "dist")]
pub mod dist {
    pub use wae_dist::*;
}

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
    pub use wae_webview::{Capability, IpcHandler, IpcOutcome, OpenOptions, Platform, Result, WebViewError};

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
