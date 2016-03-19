//! WAE 元 CLI：脚手架、构建与分发原生 npm 工具（与具体下游产品解耦）。

use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};
use wae_dist::StageNative;

#[derive(Parser, Debug)]
#[command(name = "wae", version, about = "WAE 元 CLI：脚手架 / 构建 / 分发原生 npm 工具")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 从本地模板脚手架新建工具项目。
    New {
        /// 项目目录名。
        name: String,
        /// 是否保留 GUI 静态页骨架。
        #[arg(long)]
        with_gui: bool,
        /// 输出父目录。
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// 在指定产品工作区中构建某个 Cargo 包（release）。
    Build {
        /// 产品工作区根目录（含 Cargo.toml）。
        #[arg(long)]
        project: PathBuf,
        /// 要构建的 Cargo 包名。
        #[arg(long)]
        package: String,
    },
    /// 构建并把 cdylib 暂存到 npm 平台包目录。
    Dist {
        /// 产品工作区根目录。
        #[arg(long)]
        project: PathBuf,
        /// 要构建的 Cargo 包名。
        #[arg(long)]
        package: String,
        /// 平台包输出目录。
        #[arg(long)]
        out: PathBuf,
        /// 平台包内文件名（如 `tool.win32-x64.node`）。
        #[arg(long)]
        file: String,
    },
    /// 对一个或多个 npm 包目录执行 `npm publish`（默认 dry-run）。
    Publish {
        /// npm 包目录（可重复）。
        #[arg(long = "dir", required = true)]
        dirs: Vec<PathBuf>,
        /// 真正执行 publish（否则 `--dry-run`）。
        #[arg(long)]
        execute: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::New { name, with_gui, out } => {
            if let Err(err) = cmd_new(&name, with_gui, &out) {
                eprintln!("wae new 失败：{err}");
                return ExitCode::FAILURE;
            }
        }
        Commands::Build { project, package } => {
            if let Err(err) = cmd_build(&project, &package) {
                eprintln!("wae build 失败：{err}");
                return ExitCode::FAILURE;
            }
        }
        Commands::Dist { project, package, out, file } => {
            if let Err(err) =
                wae_dist::stage_native(&StageNative { project_root: project, cargo_package: package, dest_dir: out, dest_file_name: file })
            {
                eprintln!("wae dist 失败：{err}");
                return ExitCode::FAILURE;
            }
        }
        Commands::Publish { dirs, execute } => {
            if let Err(err) = cmd_publish(&dirs, execute) {
                eprintln!("wae publish 失败：{err}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn workspace_root() -> PathBuf {
    // crates/wae → wae.rs → projects → wae 根
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap_or_else(|_| PathBuf::from("."))
}

fn cmd_new(name: &str, with_gui: bool, out: &Path) -> Result<(), String> {
    let root = workspace_root();
    let template = root.join("projects/wae.ts/templates/open-tools");
    if !template.exists() {
        return Err(format!("找不到模板目录：{}", template.display()));
    }
    let dest = out.join(name);
    if dest.exists() {
        return Err(format!("目标已存在：{}", dest.display()));
    }
    copy_dir(&template, &dest)?;
    rewrite_placeholders(&dest, name)?;
    if !with_gui {
        for gui in [dest.join("gui"), dest.join("dist").join("gui")] {
            if gui.exists() {
                let _ = std::fs::remove_dir_all(gui);
            }
        }
    }
    println!("已创建 {}（with_gui={with_gui}）", dest.display());
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &target)?;
        }
        else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn rewrite_placeholders(dir: &Path, name: &str) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            rewrite_placeholders(&path, name)?;
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str())
        else {
            continue;
        };
        if !matches!(ext, "json" | "ts" | "toml" | "md" | "html") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let next = raw.replace("{{name}}", name);
        if next != raw {
            std::fs::write(&path, next).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn cmd_build(project: &Path, package: &str) -> Result<(), String> {
    let root = project.canonicalize().map_err(|e| format!("无效的 --project：{e}"))?;
    if !root.join("Cargo.toml").exists() {
        return Err(format!("--project 缺少 Cargo.toml：{}", root.display()));
    }
    let status = Command::new("cargo").args(["build", "-p", package, "--release"]).current_dir(&root).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo build 失败（{status}）"));
    }
    println!("已在 {} 构建 -p {package}", root.display());
    Ok(())
}

fn cmd_publish(dirs: &[PathBuf], execute: bool) -> Result<(), String> {
    if dirs.is_empty() {
        return Err("请至少提供一个 --dir".into());
    }
    for pack in dirs {
        let pack = pack.canonicalize().map_err(|e| format!("无效的 --dir {}：{e}", pack.display()))?;
        if !pack.join("package.json").exists() {
            return Err(format!("--dir 缺少 package.json：{}", pack.display()));
        }
        let mut args = vec!["publish".to_string()];
        if !execute {
            args.push("--dry-run".into());
        }
        args.push("--access".into());
        args.push("public".into());
        let status = Command::new("npm").args(&args).current_dir(&pack).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("npm publish 失败：{}", pack.display()));
        }
    }
    Ok(())
}
