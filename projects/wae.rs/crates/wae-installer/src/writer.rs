//! 格式判别与 writer 开口。
//!
//! 每个格式一个纯进程内 writer；未实现的格式（`pkg` / `dmg`）与平台缺能力
//! （非 Windows 的 `msix`）一律返回 [`InstallerError::Unsupported`]，
//! 不伪造成功。禁止调用任何外部 exe/py/sh：产物字节全部由本 crate 写出，
//! Windows 侧允许经系统 DLL 的 COM/API（appxpackaging、msi）在进程内完成。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{error::Result, spec::InstallerSpec};

/// 安装包格式判别（开放集合；新增格式只加变体与 writer）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatId {
    /// Windows Installer（纯 Rust CFB + 表集 + stored-cab）。
    Msi,
    /// MSIX（系统 appxpackaging COM，仅 Windows 产出）。
    Msix,
    /// Debian 包（纯 Rust ar + ustar + stored-gzip，任何主机可产出）。
    Deb,
    /// Red Hat 包（纯 Rust lead/header + cpio newc + stored-gzip，任何主机可产出）。
    Rpm,
    /// macOS installer package（未实现，返回 Unsupported）。
    Pkg,
    /// Apple disk image（未实现，返回 Unsupported）。
    Dmg,
}

impl FormatId {
    /// 小写格式名（文件扩展名与 napi 报告共用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Msi => "msi",
            Self::Msix => "msix",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::Pkg => "pkg",
            Self::Dmg => "dmg",
        }
    }
}

/// 一次封装的成功报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteReport {
    /// 产物格式。
    pub format: FormatId,
    /// 产物路径。
    pub output: PathBuf,
    /// 产物字节数。
    pub bytes: u64,
    /// 格式细节（表清单、成员清单等），供 CLI `--json` 呈现。
    pub details: serde_json::Value,
}

/// 格式 writer 的统一开口；`write` 流式产出字节并返回写入量。
///
/// 顶层 `write_msi` / `write_deb` / `write_rpm` / `write_msix` 负责落盘与
/// [`WriteReport`] 组装；实现该 trait 的类型只关心字节流。
pub trait InstallerWriter {
    /// 本 writer 产出的格式。
    fn format(&self) -> FormatId;

    /// 把产物写入 `sink`，返回写入字节数；payload 从 `spec` 声明的路径读取。
    fn write(&self, spec: &InstallerSpec, sink: &mut dyn std::io::Write) -> Result<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_wire_is_kebab() {
        let json = serde_json::to_value([FormatId::Msi, FormatId::Msix, FormatId::Deb, FormatId::Rpm, FormatId::Pkg, FormatId::Dmg]).unwrap();
        assert_eq!(json, serde_json::json!(["msi", "msix", "deb", "rpm", "pkg", "dmg"]));
    }
}
