//! 将任意产品工作区中的原生 `cdylib` 构建并拷贝到 npm 平台包目录。
//!
//! 本 crate **不**绑定具体下游产品；调用方自行提供 Cargo 包名、产物目录与目标文件名。

#![deny(missing_docs)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

/// 分发辅助过程中的错误。
#[derive(Debug, Error)]
pub enum DistError {
    /// 可读的失败说明。
    #[error("{0}")]
    Message(String),
    /// 底层 IO 失败。
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 分发辅助的结果别名。
pub type Result<T> = std::result::Result<T, DistError>;

/// 一次原生平台包暂存请求。
#[derive(Debug, Clone)]
pub struct StageNative {
    /// 含有目标 crate 的 Cargo 工作区根目录。
    pub project_root: PathBuf,
    /// 要构建的 Cargo 包名（`-p` 参数）。
    pub cargo_package: String,
    /// 平台包输出目录（例如某 `*-win32-x64` npm 包目录）。
    pub dest_dir: PathBuf,
    /// 写入平台包内的文件名（例如 `foo.win32-x64.node`）。
    pub dest_file_name: String,
}

/// 在 `project_root` 中 `cargo build -p <package> --release`，
/// 并将找到的 cdylib 拷贝到 `dest_dir/dest_file_name`。
pub fn stage_native(req: &StageNative) -> Result<()> {
    if !req.project_root.join("Cargo.toml").exists() {
        return Err(DistError::Message(format!("项目根缺少 Cargo.toml：{}", req.project_root.display())));
    }
    if req.cargo_package.trim().is_empty() {
        return Err(DistError::Message("cargo_package 不能为空".into()));
    }
    if req.dest_file_name.trim().is_empty() {
        return Err(DistError::Message("dest_file_name 不能为空".into()));
    }

    let status = Command::new("cargo")
        .args(["build", "-p", &req.cargo_package, "--release"])
        .current_dir(&req.project_root)
        .status()
        .map_err(|e| DistError::Message(e.to_string()))?;
    if !status.success() {
        return Err(DistError::Message(format!("在 {} 构建 -p {} 失败：{status}", req.project_root.display(), req.cargo_package)));
    }

    let artifact = find_cdylib(&req.project_root.join("target/release"), &req.cargo_package)?;
    fs::create_dir_all(&req.dest_dir)?;
    let dest = req.dest_dir.join(&req.dest_file_name);
    fs::copy(&artifact, &dest)?;
    println!("已暂存 {} → {}", artifact.display(), dest.display());
    Ok(())
}

fn find_cdylib(dir: &Path, cargo_package: &str) -> Result<PathBuf> {
    let stem = cargo_package.replace('-', "_");
    let candidates = [
        dir.join(format!("{stem}.dll")),
        dir.join(format!("{stem}.node")),
        dir.join(format!("lib{stem}.so")),
        dir.join(format!("lib{stem}.dylib")),
    ];
    for c in candidates {
        if c.exists() {
            return Ok(c);
        }
    }
    Err(DistError::Message(format!("在 {} 下未找到包 `{cargo_package}` 的原生产物（期望 stem=`{stem}`）", dir.display())))
}
