//! macOS WKWebView 后端：[`MacosBackend`]。
//!
//! 非 macOS 目标上本 crate 不提供任何能力，且不会被门面选中。

#![deny(missing_docs)]

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::MacosBackend;
