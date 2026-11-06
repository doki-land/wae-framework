//! Linux WebKitGTK 4.1 + GTK3 后端：[`LinuxBackend`]。
//!
//! 非 Linux 目标上本 crate 不提供任何能力，且不会被门面选中。

#![deny(missing_docs)]

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::LinuxBackend;
