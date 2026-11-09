//! 用系统关联程序启动文件。
//!
//! 三个桌面平台暴露同一签名（[`open_path`]），差异只在本 crate 的
//! `#[cfg(target_os)]` 模块内部：Windows 走 `ShellExecuteW("open")`，
//! macOS 走 `open`，Linux 走 `xdg-open`。启动器缺失或失败一律映射为
//! [`ShellError`]，下游按字段判别，不得写平台 `cfg`。
//!
//! 本 crate 不等待目标进程退出：`open_path` 成功仅表示启动器已接受请求。

#![deny(missing_docs)]

use std::path::Path;

pub use wae_webview::Platform;

#[cfg(target_os = "windows")]
mod win32;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
mod linux;

/// [`open_path`] 的稳定失败分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellError {
    /// 目标文件不存在。
    Missing {
        /// 检查失败的路径原文。
        path: String,
    },
    /// 当前编译目标没有启动器实现。
    Unsupported {
        /// 被判定的平台。
        platform: Platform,
    },
    /// 启动器拒绝或无法启动。
    LaunchFailed {
        /// 稳定启动器名：`ShellExecuteW` / `open` / `xdg-open`。
        launcher: &'static str,
        /// 底层错误文本（Win32 返回值或 io 错误）。
        message: String,
    },
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellError::Missing { path } => write!(formatter, "shell file is missing: {path}"),
            ShellError::Unsupported { platform } => write!(formatter, "shell open is unsupported on {platform:?}"),
            ShellError::LaunchFailed { launcher, message } => write!(formatter, "shell launcher {launcher} failed: {message}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// 用系统关联程序打开 `path`，立即返回，不等待目标进程退出。
///
/// 只接受真实存在的文件；目录与其它路径一律 [`ShellError::Missing`]，
/// 调用方（如 OpenZipper 宿主）应只传入自己刚产出的临时文件。
#[cfg(target_os = "windows")]
pub fn open_path(path: &Path) -> Result<(), ShellError> {
    win32::open_path(path)
}

/// 用系统关联程序打开 `path`，立即返回，不等待目标进程退出。
#[cfg(target_os = "macos")]
pub fn open_path(path: &Path) -> Result<(), ShellError> {
    macos::open_path(path)
}

/// 用系统关联程序打开 `path`，立即返回，不等待目标进程退出。
#[cfg(target_os = "linux")]
pub fn open_path(path: &Path) -> Result<(), ShellError> {
    linux::open_path(path)
}

/// 非桌面目标在编译期显式拒绝，不提供只会返回错误的 stub。
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn open_path(_path: &Path) -> Result<(), ShellError> {
    compile_error!("`wae-shell` 只支持 Windows / macOS / Linux；其它目标没有系统启动器");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_reported_before_any_launcher_runs() {
        let path = Path::new("wae-shell-definitely-missing-file.bin");
        assert_eq!(open_path(path), Err(ShellError::Missing { path: path.display().to_string() }));
    }

    #[test]
    fn error_display_carries_the_stable_launcher_name() {
        let error = ShellError::LaunchFailed { launcher: "xdg-open", message: "not found".into() };
        assert!(error.to_string().contains("xdg-open"));
        assert!(error.to_string().contains("not found"));
    }
}
