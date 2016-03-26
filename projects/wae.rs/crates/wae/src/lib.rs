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

/// `wae-webview-win32`（feature `webview`，默认开启）。
#[cfg(feature = "webview")]
pub mod webview {
    pub use wae_webview_win32::*;
}
