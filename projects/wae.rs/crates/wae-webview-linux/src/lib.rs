//! Linux WebKitGTK 4.1 + GTK3 后端：[`LinuxBackend`]。
//!
//! 非 Linux 目标上本 crate 不提供任何能力，且不会被门面选中。
//!
//! `OpenOptions.accept_file_drop` 在本后端接收但未接线（`Capability::FileDrop`
//! 占位）；接线时推送的消息形状必须与 win32 一致：`{"type":"fileDrop","paths":[...]}`。

#![deny(missing_docs)]

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::LinuxBackend;
