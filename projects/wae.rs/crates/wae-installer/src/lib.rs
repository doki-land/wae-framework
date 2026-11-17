//! WAE **安装包封装**：把下游应用的构建产物（exe + 静态 dist）封装成
//! 系统规定的安装包，而不是压缩包。
//!
//! 硬约束（用户 2026-11-17 裁决，vmz 系列原则）：**不调用任何外部
//! exe/py/sh**（makensis、makeappx、dpkg-deb、rpmbuild、productbuild 等
//! 全部排除）。所有产物字节由进程内 Rust 代码写出；Windows 侧允许使用
//! 系统 DLL 的 COM/API（`appxpackaging.dll`、`msi.dll`）在进程内完成，
//! 与 `wae-webview-win32` 使用 WebView2 COM 同一口径。
//!
//! 本轮格式：
//!
//! | 格式 | 实现 | 产出主机 |
//! |------|------|----------|
//! | `msi` | 纯 Rust：CFB compound file + 最小表集 + 内嵌 stored-cab | 任意 |
//! | `deb` | 纯 Rust：ar + ustar + stored-gzip | 任意 |
//! | `rpm` | 纯 Rust：lead + signature/header + cpio newc + stored-gzip | 任意 |
//! | `msix` | 系统 `appxpackaging.dll` COM，可选 crypt32 自签 | 仅 Windows |
//! | `pkg` / `dmg` | 未实现 | 返回结构化 `Unsupported`，不伪造成功 |
//!
//! MSI 合同（v0）：静默优先——保证 `msiexec /qn` 与 `MsiInstallProduct`
//! 进程内 API 可安装；交互 UI 表集（`UI`/`Dialog`/`Control`…）不在本轮，
//! 双击安装可能没有进度界面，如实声明。payload 文件名限 ASCII（MSI
//! 字符串池按 ANSI 代码页）。`UpgradeCode` 一旦发布永不变更。

#![deny(missing_docs)]

// 容器与字节原语；由各格式 writer 接线使用。
#[allow(dead_code)]
mod cab;
#[allow(dead_code)]
mod cfb;
#[allow(dead_code)]
mod crc32;
pub mod error;
pub mod spec;
pub mod writer;

pub use error::{InstallerError, Platform, Result, codes, current_platform};
pub use spec::{
    ArpMeta, DebOptions, DirKey, InstallerSpec, MsiOptions, MsixOptions, PayloadFile, ProductMeta, RpmOptions, Scope, Shortcut,
    is_exact_version, is_guid_text, spec_codes,
};
pub use writer::{FormatId, InstallerWriter, WriteReport};
