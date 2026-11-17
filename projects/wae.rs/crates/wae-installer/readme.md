# wae-installer

WAE 安装包封装：把下游应用的构建产物（exe + 静态 dist）封装成系统规定的安装包，而不是压缩包。

- **硬约束**：不调用任何外部 exe/py/sh。所有产物字节由进程内 Rust 写出；Windows 侧允许经系统 DLL 的 COM/API（`appxpackaging.dll`、`msi.dll`）在进程内完成。
- **格式**：`msi`（纯 Rust CFB + 最小表集 + 内嵌 stored-cab）、`deb`（纯 Rust ar + ustar + stored-gzip）、`rpm`（纯 Rust lead/header + cpio newc）、`msix`（系统 Appx COM，仅 Windows）；`pkg` / `dmg` 未实现，返回结构化 `Unsupported`，不伪造成功。
- **MSI 合同（v0）**：静默优先——保证 `msiexec /qn` 与 `MsiInstallProduct` 进程内 API 可安装；交互 UI 表集不在本轮，双击安装可能没有进度界面。payload 文件名限 ASCII。
- **UpgradeCode 纪律**：`upgradeCode` 一旦发布永不变更（换码 = 旧包永不清理）。缺省由确定性派生，作者必须回填固定到 `wae.config.ts`。
- **签名如实声明**：免签可被 `msiexec` 安装 ≠ 终端用户无 SmartScreen 提示；MSIX 自签仅证明结构合法，不进信任链。

## 用法

```rust
use wae_installer::{InstallerSpec, MsiOptions, write_msi};

let report = write_msi(&spec, &MsiOptions::default())?;
```

验证一律进程内：`verify_msi` 经 `MsiOpenDatabase` / `MsiViewExecute` 逐表回读，管理安装解包用 `MsiInstallProduct(ACTION=ADMIN)`，不 spawn 任何 exe。
