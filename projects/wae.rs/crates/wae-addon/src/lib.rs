//! WAE 原生 addon：把门面能力与审计入口暴露给 Node。
//!
//! 使用自研薄 Node-API C ABI（`wae-napi`，dyn-symbols），不依赖
//! `napi` / `napi-derive` / `napi-build`。构建产物由平台包分发，
//! 类型面由 `@wae/wae` 的 `types` 构建步从 `__waeTypes` 生成。
//!
//! 本 crate 同时是宏的「自食」样例：导出函数直接写领域类型，
//! 参数与返回值经 `FromJs` / `ToJs` 转换，不再手写字段提取与 JSON 桥。

use std::{
    cell::RefCell,
    collections::BTreeMap,
    env::consts,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use wae::{
    CallContext, Error, Result,
    audit::{ActorKind, Auditor, Mode, Query, napi_call},
    class, function, object, register_module,
    webview::{IpcHandler, IpcOutcome, IpcSink, OpenOptions},
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

/// 打开 harness 的参数；`url` 与 `contentDir` 必须恰好给一个。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HarnessOpenOptions {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content_dir: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    width: Option<i32>,
    #[serde(default)]
    height: Option<i32>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    accept_file_drop: Option<bool>,
}

/// 把 addon 收到的参数转成 WebView 选项；缺省值沿用平台合同。
fn harness_open_options(input: HarnessOpenOptions) -> Result<OpenOptions> {
    let title = input.title.unwrap_or_else(|| "WAE harness".to_string());
    let mut options = match (input.url, input.content_dir) {
        (Some(url), None) => OpenOptions::from_url(url, title),
        (None, Some(content_dir)) => OpenOptions::from_content_dir(content_dir, title),
        _ => {
            return Err(Error::invalid_input("harnessOpen needs exactly one of `url` or `contentDir`"));
        }
    };
    if let Some(width) = input.width {
        options.width = width;
    }
    if let Some(height) = input.height {
        options.height = height;
    }
    options.hidden = input.hidden.unwrap_or(false);
    options.accept_file_drop = input.accept_file_drop.unwrap_or(false);
    Ok(options)
}

/// 页面 → 宿主消息先入队，由 JS 经 `harnessTakeIpc` 取走，避免跨线程回调 JS。
struct QueueHandler(Arc<Mutex<Vec<String>>>);

impl IpcHandler for QueueHandler {
    fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
        self.0.lock().expect("harness ipc queue").push(message.to_owned());
        IpcOutcome::Ignored
    }
}

struct HarnessEntry {
    harness: wae::harness::PlatformHarness,
    ipc: Arc<Mutex<Vec<String>>>,
}

/// JS 线程私有的 harness 句柄表；句柄只在本线程有效，不跨 worker 复用。
#[derive(Default)]
struct HarnessRegistry {
    next: u64,
    entries: BTreeMap<u64, HarnessEntry>,
}

thread_local! {
    static HARNESSES: RefCell<HarnessRegistry> = RefCell::new(HarnessRegistry::default());
}

fn harness_error(error: wae::harness::HarnessError) -> Error {
    Error::new(error.to_string()).code(error.code())
}

fn with_harness<T>(handle: u64, run: impl FnOnce(&mut wae::harness::PlatformHarness) -> Result<T>) -> Result<T> {
    HARNESSES.with(|registry| {
        let mut registry = registry.borrow_mut();
        let entry = registry.entries.get_mut(&handle).ok_or_else(|| Error::invalid_input(format!("unknown harness handle {handle}")))?;
        run(&mut entry.harness)
    })
}

/// 打开一个隐藏或可见的 WebView harness，返回句柄。
#[function(js_name = "harnessOpen")]
fn harness_open(options: String) -> Result<u64> {
    let input: HarnessOpenOptions = serde_json::from_str(&options).map_err(|error| Error::invalid_input(error.to_string()))?;
    let open = harness_open_options(input)?;
    // 当前 WebView 宿主一个进程只拥有一个窗口；提前给出稳定错误，而不是让
    // controller 创建阶段抛出难以解释的 TaskCanceled。
    if HARNESSES.with(|registry| !registry.borrow().entries.is_empty()) {
        return Err(Error::new("the WebView host owns one harness window per process").code("harness_single_instance"));
    }
    let ipc = Arc::new(Mutex::new(Vec::new()));
    let harness = wae::harness::PlatformHarness::open(open, Box::new(QueueHandler(Arc::clone(&ipc)))).map_err(harness_error)?;
    Ok(HARNESSES.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.next += 1;
        let handle = registry.next;
        registry.entries.insert(handle, HarnessEntry { harness, ipc });
        handle
    }))
}

/// 执行页面脚本，返回 JSON 文本结果。
#[function(js_name = "harnessEval")]
fn harness_eval(handle: u64, script: String) -> Result<String> {
    let value = with_harness(handle, |harness| harness.eval(&script).map_err(harness_error))?;
    serde_json::to_string(&value).map_err(|error| Error::new(error.to_string()))
}

/// 轮询脚本直到结果为 `true`，超时返回结构化错误。
#[function(js_name = "harnessWaitFor")]
fn harness_wait_for(handle: u64, script: String, timeout_ms: u64) -> Result<String> {
    let value = with_harness(handle, |harness| harness.wait_for(&script, Duration::from_millis(timeout_ms.max(1))).map_err(harness_error))?;
    serde_json::to_string(&value).map_err(|error| Error::new(error.to_string()))
}

/// 截图写入 `path`，返回实际文件路径。
#[function(js_name = "harnessScreenshot")]
fn harness_screenshot(handle: u64, path: String) -> Result<String> {
    let written = with_harness(handle, |harness| harness.screenshot(std::path::Path::new(&path)).map_err(harness_error))?;
    Ok(written.display().to_string())
}

/// 驱动一次消息泵；窗口已销毁返回 `false`。
#[function(js_name = "harnessPump")]
fn harness_pump(handle: u64) -> Result<bool> {
    with_harness(handle, |harness| if harness.host_mut().pump() { Ok(true) } else { Ok(false) })
}

/// 向页面投递一条字符串消息。
#[function(js_name = "harnessPost")]
fn harness_post(handle: u64, text: String) -> Result<()> {
    with_harness(handle, |harness| harness.host_mut().post_message(&text).map_err(|error| Error::new(error.to_string())))
}

/// 取走页面发来的全部 IPC 消息，返回 JSON 字符串数组。
#[function(js_name = "harnessTakeIpc")]
fn harness_take_ipc(handle: u64) -> Result<String> {
    let messages: Vec<String> = HARNESSES.with(|registry| -> Result<Vec<String>> {
        let registry = registry.borrow();
        let entry = registry.entries.get(&handle).ok_or_else(|| Error::invalid_input(format!("unknown harness handle {handle}")))?;
        let mut queue = entry.ipc.lock().expect("harness ipc queue");
        Ok(queue.drain(..).collect::<Vec<String>>())
    })?;
    serde_json::to_string(&messages).map_err(|error| Error::new(error.to_string()))
}

/// 关闭并移除 harness 句柄；重复调用视为失败，避免静默吞掉生命周期错误。
#[function(js_name = "harnessClose")]
fn harness_close(handle: u64) -> Result<()> {
    let mut entry = HARNESSES.with(|registry| registry.borrow_mut().entries.remove(&handle));
    match entry.take() {
        Some(mut entry) => entry.harness.close().map_err(harness_error),
        None => Err(Error::invalid_input(format!("unknown harness handle {handle}"))),
    }
}

register_module![version, capabilities, probe, probe_async, audit_query, harness_open, harness_eval, harness_wait_for, harness_screenshot, harness_pump, harness_post, harness_take_ipc, harness_close; Counter];
