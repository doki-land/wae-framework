//! WAE 原生 addon：把门面能力与审计入口暴露给 Node。
//!
//! 使用自研薄 Node-API C ABI（`wae-napi`，dyn-symbols），不依赖
//! `napi` / `napi-derive` / `napi-build`。构建产物由平台包分发，
//! 类型面由 `@wae/wae` 的 `types` 构建步从 `__waeTypes` 生成。
//!
//! 本 crate 同时是宏的「自食」样例：导出函数直接写领域类型，
//! 参数与返回值经 `FromJs` / `ToJs` 转换，不再手写字段提取与 JSON 桥。

use std::{env::consts, sync::OnceLock};

use wae::{
    CallContext, Error, Result,
    audit::{ActorKind, Auditor, Mode, Query, napi_call},
    class, function, object, register_module,
};

/// addon 级审计器：应用名固定为 `wae`，版本随构建。
fn auditor() -> &'static Auditor {
    static AUDITOR: OnceLock<Auditor> = OnceLock::new();
    AUDITOR.get_or_init(|| Auditor::from_env("wae", env!("CARGO_PKG_VERSION")))
}

/// 平台与构建信息。
#[object]
#[derive(Debug, Clone)]
struct CapabilitiesReport {
    /// Node 平台 id（`windows` / `macos` / `linux`）。
    platform: String,
    /// CPU 架构（`x86_64` / `aarch64`）。
    arch: String,
    /// 审计设施是否可用。
    audit: bool,
    /// addon 版本。
    version: String,
}

/// 最小能力探针结果。
#[object]
#[derive(Debug, Clone)]
struct ProbeReport {
    /// 探针是否成功。
    ok: bool,
    /// Node 平台 id。
    platform: String,
    /// CPU 架构。
    arch: String,
}

/// 审计查询过滤条件。
#[object]
#[derive(Debug, Clone, Default)]
struct AuditFilter {
    /// `user` 或 `agent`。
    kind: Option<String>,
    /// actor id。
    id: Option<String>,
    /// 能力名。
    capability: Option<String>,
    /// 起始时间（含），Unix 毫秒。
    from_ms: Option<u64>,
    /// 结束时间（含），Unix 毫秒。
    to_ms: Option<u64>,
    /// 最多返回条数。
    limit: Option<u64>,
}

/// 当前 addon 版本。
#[function(js_name = "version")]
fn version() -> Result<String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

/// 本平台能力自检：平台、架构与审计设施是否可用。
#[function(js_name = "capabilities")]
fn capabilities() -> Result<CapabilitiesReport> {
    Ok(CapabilitiesReport {
        platform: consts::OS.to_string(),
        arch: consts::ARCH.to_string(),
        audit: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// 最小能力探针；调用经 `wae-audit` 记录，供加载冒烟与审计接线验证。
#[function(js_name = "probe")]
fn probe(ctx: &CallContext) -> Result<ProbeReport> {
    let report = ProbeReport { ok: true, platform: consts::OS.to_string(), arch: consts::ARCH.to_string() };
    napi_call(auditor(), "wae.native.probe", Mode::Execute)
        .targets(0, "native addon probe")
        .run(|| wae::ToJs::to_js(report.clone(), ctx.env()))?;
    Ok(report)
}

/// 异步探针：在 Node 托管线程池上运行，用来验证 async 导出与 Promise 结算。
///
/// `delayMs` 只用于观测线程池行为（事件循环是否被阻塞），上限 5 秒。
#[function(js_name = "probeAsync", async)]
fn probe_async(delay_ms: Option<u64>) -> Result<ProbeReport> {
    if let Some(delay) = delay_ms {
        std::thread::sleep(std::time::Duration::from_millis(delay.min(5_000)));
    }
    Ok(ProbeReport { ok: true, platform: consts::OS.to_string(), arch: consts::ARCH.to_string() })
}

/// 查询审计记录，返回 JSON 数组文本。
#[function(js_name = "auditQuery")]
fn audit_query(filter: Option<AuditFilter>) -> Result<String> {
    let filter = filter.unwrap_or_default();
    let query = Query {
        actor_kind: filter.kind.as_deref().map(actor_kind),
        actor_id: filter.id,
        capability: filter.capability,
        from_ms: filter.from_ms,
        to_ms: filter.to_ms,
        limit: filter.limit.map(|value| value as usize),
    };
    auditor().query_json(&query).map_err(|error| Error::new(error.to_string()))
}

/// 计数器：验证类导出、实例封装与 finalizer。
#[derive(Debug, Default)]
struct Counter {
    value: u32,
}

#[class]
impl Counter {
    /// 以起始值创建实例。
    #[constructor]
    fn new(start: u32) -> Result<Self> {
        Ok(Self { value: start })
    }

    /// 累加并返回新值。
    #[method]
    fn add(&mut self, n: u32) -> Result<u32> {
        self.value += n;
        Ok(self.value)
    }

    /// 当前值。
    #[getter]
    fn value(&self) -> Result<u32> {
        Ok(self.value)
    }
}

/// `kind` 字符串映射到身份类型；未知值按 `user` 处理，与旧行为一致。
fn actor_kind(value: &str) -> ActorKind {
    if value.eq_ignore_ascii_case("agent") { ActorKind::Agent } else { ActorKind::User }
}

register_module![version, capabilities, probe, probe_async, audit_query; Counter];
