//! Win32 + WebView2 后端实现：[`Win32Backend`]。
//!
//! 非 Windows 目标上本 crate 不提供任何能力，且不会被门面选中。

#![deny(missing_docs)]

#[cfg(windows)]
mod console;

#[cfg(windows)]
mod dpi;

#[cfg(windows)]
mod win32;

#[cfg(windows)]
pub use win32::Win32Backend;
