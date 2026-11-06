//! WAE 原生 addon：把门面能力与审计入口暴露给 Node。
//!
//! 使用自研薄 Node-API C ABI（`wae-napi`，dyn-symbols），不依赖
//! `napi` / `napi-derive` / `napi-build`。构建产物由平台包分发，
//! 加载与失败形态见 `@wae/wae/native`。

use std::{env::consts, sync::OnceLock};

use wae::{
    CallContext, JsValue, Result,
    audit::{Auditor, Mode, napi_call, query_from_napi},
    function, register_module,
};

mod packaging;

use packaging::{
    WAE_NAPI_EXPORT_FIND_ARTIFACT, WAE_NAPI_EXPORT_STAGE_ARTIFACT, WAE_NAPI_EXPORT_VALIDATE_PLATFORM_PACKAGE,
};

/// 原子分发能力的 JS 导出名，供 `capabilities()` 自检。
const ATOMIC_CAPABILITIES: [&str; 3] = ["findArtifact", "stageArtifact", "validatePlatformPackage"];

/// addon 级审计器：应用名固定为 `wae`，版本随构建。
fn auditor() -> &'static Auditor {
    static AUDITOR: OnceLock<Auditor> = OnceLock::new();
    AUDITOR.get_or_init(|| Auditor::from_env("wae", env!("CARGO_PKG_VERSION")))
}

/// 当前 addon 版本。
#[function(js_name = "version")]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string(env!("CARGO_PKG_VERSION"))
}

/// 本平台能力自检：平台、架构与审计设施是否可用。
#[function(js_name = "capabilities")]
fn capabilities(ctx: CallContext) -> Result<JsValue> {
    let capabilities = ctx.env().object()?;
    capabilities.set_named("platform", ctx.env().string(consts::OS)?)?;
    capabilities.set_named("arch", ctx.env().string(consts::ARCH)?)?;
    capabilities.set_named("audit", ctx.env().boolean(true)?)?;
    capabilities.set_named("version", ctx.env().string(env!("CARGO_PKG_VERSION"))?)?;
    let atomic = ctx.env().array_with_length(ATOMIC_CAPABILITIES.len())?;
    for (index, name) in ATOMIC_CAPABILITIES.iter().enumerate() {
        atomic.set_element(index as u32, ctx.env().string(name)?)?;
    }
    capabilities.set_named("atomic", atomic)?;
    Ok(capabilities)
}

/// 最小能力探针；调用经 `wae-audit` 记录，供加载冒烟与审计接线验证。
#[function(js_name = "probe")]
fn probe(ctx: CallContext) -> Result<JsValue> {
    napi_call(auditor(), "wae.native.probe", Mode::Execute)
        .targets(0, "native addon probe")
        .run(|| {
            let report = ctx.env().object()?;
            report.set_named("ok", ctx.env().boolean(true)?)?;
            report.set_named("platform", ctx.env().string(consts::OS)?)?;
            report.set_named("arch", ctx.env().string(consts::ARCH)?)?;
            Ok(report)
        })
}

/// 查询审计记录，返回 JSON 数组文本。
#[function(js_name = "auditQuery")]
fn audit_query(ctx: CallContext) -> Result<JsValue> {
    query_from_napi(&ctx, auditor())
}

register_module![version, capabilities, probe, audit_query, find_artifact, stage_artifact, validate_platform_package];
