//! 原子分发能力：找产物、暂存、平台包结构校验。
//!
//! 这些函数只接受调用方传入的显式路径与包名，不内嵌任何项目名、仓路径或编排逻辑。
//! 预期失败返回 `{ ok: false, code, message }`，不抛异常。

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use wae::{CallContext, JsValue, Result, function};

/// 未找到原生构建产物。
const ARTIFACT_MISSING: &str = "artifact_missing";
/// 平台包结构不符合分发合同。
const PLATFORM_PACKAGE_INVALID: &str = "platform_package_invalid";
/// 参数缺失或形状错误。
const INVALID_ARGUMENT: &str = "invalid_argument";
/// 文件系统操作失败。
const IO_ERROR: &str = "io_error";

/// 平台包校验结果：`(name, version, os, cpu, files, main)`。
type PlatformPackage = (String, String, Vec<String>, Vec<String>, Vec<String>, String);

struct OpError {
    code: &'static str,
    message: String,
}

impl OpError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

fn ok(ctx: &CallContext, value: JsValue) -> Result<JsValue> {
    let envelope = ctx.env().object()?;
    envelope.set_named("ok", ctx.env().boolean(true)?)?;
    envelope.set_named("value", value)?;
    Ok(envelope)
}

fn fail(ctx: &CallContext, error: OpError) -> Result<JsValue> {
    let envelope = ctx.env().object()?;
    envelope.set_named("ok", ctx.env().boolean(false)?)?;
    envelope.set_named("code", ctx.env().string(error.code)?)?;
    envelope.set_named("message", ctx.env().string(&error.message)?)?;
    Ok(envelope)
}

fn required_string(ctx: &CallContext, name: &str) -> std::result::Result<String, OpError> {
    let spec = ctx.arg(0).map_err(|err| OpError::new(INVALID_ARGUMENT, format!("缺少参数对象：{err}")))?;
    let value = wae::optional_string_prop(spec, name).map_err(|err| OpError::new(INVALID_ARGUMENT, format!("读取 `{name}` 失败：{err}")))?;
    match value {
        Some(text) if !text.trim().is_empty() => Ok(text),
        _ => Err(OpError::new(INVALID_ARGUMENT, format!("`{name}` 必须是非空字符串"))),
    }
}

fn string_array(ctx: &CallContext, entries: &[String]) -> Result<JsValue> {
    let array = ctx.env().array_with_length(entries.len())?;
    for (index, entry) in entries.iter().enumerate() {
        array.set_element(index as u32, ctx.env().string(entry)?)?;
    }
    Ok(array)
}

/// 按 Cargo 包名在给定 release 目录中查找 cdylib，返回 `{ ok, value: { path } }`。
#[function(js_name = "findArtifact")]
pub(crate) fn find_artifact(ctx: CallContext) -> Result<JsValue> {
    let outcome = (|| -> std::result::Result<String, OpError> {
        let release_dir = required_string(&ctx, "releaseDir")?;
        let package = required_string(&ctx, "package")?;
        let artifact = wae_dist::find_cdylib(Path::new(&release_dir), &package)
            .map_err(|err| OpError::new(ARTIFACT_MISSING, format!("在 `{release_dir}` 下未找到包 `{package}` 的原生产物：{err}")))?;
        Ok(artifact.display().to_string())
    })();

    match outcome {
        Ok(path) => {
            let value = ctx.env().object()?;
            value.set_named("path", ctx.env().string(&path)?)?;
            ok(&ctx, value)
        }
        Err(error) => fail(&ctx, error),
    }
}

/// 把已构建的产物复制为平台包内的 `.node` 文件，返回 `{ ok, value: { path } }`。
#[function(js_name = "stageArtifact")]
pub(crate) fn stage_artifact(ctx: CallContext) -> Result<JsValue> {
    let outcome = (|| -> std::result::Result<String, OpError> {
        let artifact = required_string(&ctx, "artifact")?;
        let dest_dir = required_string(&ctx, "destDir")?;
        let file_name = required_string(&ctx, "fileName")?;
        if !file_name.ends_with(".node") {
            return Err(OpError::new(INVALID_ARGUMENT, "`fileName` 必须以 `.node` 结尾"));
        }
        let source = PathBuf::from(&artifact);
        if !source.is_file() {
            return Err(OpError::new(ARTIFACT_MISSING, format!("产物不存在：{artifact}")));
        }
        fs::create_dir_all(&dest_dir).map_err(|err| OpError::new(IO_ERROR, format!("无法创建 `{dest_dir}`：{err}")))?;
        let dest = Path::new(&dest_dir).join(&file_name);
        fs::copy(&source, &dest).map_err(|err| OpError::new(IO_ERROR, format!("复制到 `{}` 失败：{err}", dest.display())))?;
        Ok(dest.display().to_string())
    })();

    match outcome {
        Ok(path) => {
            let value = ctx.env().object()?;
            value.set_named("path", ctx.env().string(&path)?)?;
            ok(&ctx, value)
        }
        Err(error) => fail(&ctx, error),
    }
}

fn is_exact_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn string_field(manifest: &Value, name: &str) -> String {
    manifest.get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn string_array_field(manifest: &Value, name: &str) -> Vec<String> {
    manifest
        .get(name)
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn is_metadata_entry(entry: &str) -> bool {
    let lowered = entry.to_ascii_lowercase();
    matches!(lowered.as_str(), "readme.md" | "license.md" | "license" | "package.json")
}

/// 校验平台包的 `os` / `cpu` / `files` / `main` 结构，并要求产物真实存在。
///
/// 返回 `{ ok, value: { name, version, os, cpu, files, main } }`。
#[function(js_name = "validatePlatformPackage")]
pub(crate) fn validate_platform_package(ctx: CallContext) -> Result<JsValue> {
    let outcome = (|| -> std::result::Result<PlatformPackage, OpError> {
        let dir = required_string(&ctx, "dir")?;
        let manifest_path = Path::new(&dir).join("package.json");
        let text = fs::read_to_string(&manifest_path)
            .map_err(|err| OpError::new(PLATFORM_PACKAGE_INVALID, format!("无法读取 {}：{err}", manifest_path.display())))?;
        let manifest: Value = serde_json::from_str(&text)
            .map_err(|err| OpError::new(PLATFORM_PACKAGE_INVALID, format!("{} 不是合法 JSON：{err}", manifest_path.display())))?;

        let name = string_field(&manifest, "name");
        let version = string_field(&manifest, "version");
        if name.is_empty() {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("{} 缺少 name", manifest_path.display())));
        }
        if !is_exact_version(&version) {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 的 version 必须是精确 X.Y.Z，实际为 `{version}`")));
        }

        let os = string_array_field(&manifest, "os");
        let cpu = string_array_field(&manifest, "cpu");
        if os.is_empty() {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 必须声明非空 `os`")));
        }
        if cpu.is_empty() {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 必须声明非空 `cpu`")));
        }

        let files = string_array_field(&manifest, "files");
        if files.is_empty() {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 必须声明非空 `files`")));
        }
        for entry in &files {
            if !entry.ends_with(".node") && !is_metadata_entry(entry) {
                return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 的 files 条目 `{entry}` 既不是原生产物也不是包元数据")));
            }
        }

        let main = string_field(&manifest, "main");
        let file = files.iter().find(|entry| entry.ends_with(".node")).cloned().unwrap_or_default();
        if file.is_empty() {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 的 files 中没有 `.node` 产物")));
        }
        if main != file {
            return Err(OpError::new(PLATFORM_PACKAGE_INVALID, format!("`{name}` 的 main 必须是 `{file}`")));
        }

        let artifact = Path::new(&dir).join(&file);
        if !artifact.is_file() {
            return Err(OpError::new(ARTIFACT_MISSING, format!("`{name}` 缺少产物：{}", artifact.display())));
        }

        Ok((name, version, os, cpu, files, file))
    })();

    match outcome {
        Ok((name, version, os, cpu, files, file)) => {
            let value = ctx.env().object()?;
            value.set_named("name", ctx.env().string(&name)?)?;
            value.set_named("version", ctx.env().string(&version)?)?;
            value.set_named("os", string_array(&ctx, &os)?)?;
            value.set_named("cpu", string_array(&ctx, &cpu)?)?;
            value.set_named("files", string_array(&ctx, &files)?)?;
            value.set_named("main", ctx.env().string(&file)?)?;
            ok(&ctx, value)
        }
        Err(error) => fail(&ctx, error),
    }
}
