//! WAE 审计记录：固定 schema、脱敏、append-only JSONL 与查询。
//!
//! 所有 WAE 系产品的调用都应记录为可归因到具体 `user` 或 `agent` 的一条记录。
//! 记录点由门面与 napi 入口提供，应用只补领域字段（能力名、参数、目标、错误码）。
//!
//! `actor` 只表达调用者身份，角色与权限判定属于独立授权模型：授权失败记录为
//! `rejected`，不得用 `role` 顶替 `actor`。

#![deny(missing_docs)]

mod sanitize;
mod store;

use std::{
    env,
    path::PathBuf,
    process,
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use sanitize::{REDACTED, sanitize_params};
pub use store::{AuditError, AuditResult, Store};

/// 审计记录合同标识。
pub const SCHEMA: &str = "wae.audit/v0";

/// 默认单文件轮转阈值（8 MiB）。
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 调用者身份类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// 人类用户。
    User,
    /// agent 或自动化调用方。
    Agent,
}

/// 调用者身份。授权判定不得写入本类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    /// 身份类型（`user` / `agent`）。
    pub kind: ActorKind,
    /// 稳定身份 id。
    pub id: String,
    /// 入口来源（例如 `cli`、`gui`、`plugin`、`test`）。
    pub source: String,
}

impl Actor {
    /// 按环境变量解析调用者身份。
    ///
    /// 读取 `WAE_ACTOR_KIND`、`WAE_ACTOR_ID`、`WAE_ACTOR_SOURCE`。未设置时
    /// `kind` 为 `user`，`id` 取本机用户名，`source` 为 `local`。
    pub fn from_env() -> Self {
        let kind = match env::var("WAE_ACTOR_KIND") {
            Ok(value) if value.eq_ignore_ascii_case("agent") => ActorKind::Agent,
            _ => ActorKind::User,
        };
        let id = env_text("WAE_ACTOR_ID").unwrap_or_else(|| match kind {
            ActorKind::User => os_user().unwrap_or_else(|| "unknown".into()),
            ActorKind::Agent => "unknown".into(),
        });
        let source = env_text("WAE_ACTOR_SOURCE").unwrap_or_else(|| "local".into());
        Self { kind, id, source }
    }
}

/// 调用模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 只预演，不产生实际修改。
    DryRun,
    /// 实际执行。
    Execute,
}

/// 调用结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// 成功。
    Success,
    /// 执行失败。
    Failure,
    /// 被授权或前置校验拒绝。
    Rejected,
}

/// 调用目标数量与摘要。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Targets {
    /// 目标数量。
    pub count: u64,
    /// 目标摘要。
    pub summary: String,
}

impl Targets {
    /// 构造目标数量与摘要。
    pub fn new(count: u64, summary: impl Into<String>) -> Self {
        Self { count, summary: summary.into() }
    }
}

/// 一条审计记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// 固定为 [`SCHEMA`]。
    pub schema: String,
    /// Unix 毫秒时间戳。
    pub timestamp_ms: u64,
    /// 调用者身份。
    pub actor: Actor,
    /// 应用名。
    pub app: String,
    /// 应用版本。
    pub app_version: String,
    /// 能力名。
    pub capability: String,
    /// 脱敏后的业务参数。
    pub params: Value,
    /// 调用模式。
    pub mode: Mode,
    /// 目标数量与摘要。
    pub targets: Targets,
    /// 调用结果（JSON 字段名为 `result`）。
    #[serde(rename = "result")]
    pub outcome: Outcome,
    /// 失败或被拒绝时的稳定错误码。
    pub error_code: Option<String>,
    /// 调用耗时（毫秒）。
    pub duration_ms: u64,
    /// 主机标识。
    pub host: String,
    /// 会话关联 id。
    pub session: String,
}

/// 审计记录查询条件；字段为 `None` 时不参与过滤。
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// 只保留该身份类型。
    pub actor_kind: Option<ActorKind>,
    /// 只保留该 actor id。
    pub actor_id: Option<String>,
    /// 起始时间（含），Unix 毫秒。
    pub from_ms: Option<u64>,
    /// 结束时间（含），Unix 毫秒。
    pub to_ms: Option<u64>,
    /// 只保留该能力名。
    pub capability: Option<String>,
    /// 最多返回的记录数。
    pub limit: Option<usize>,
}

impl Query {
    fn matches(&self, entry: &Entry) -> bool {
        if let Some(kind) = self.actor_kind
            && entry.actor.kind != kind
        {
            return false;
        }
        if let Some(id) = &self.actor_id
            && &entry.actor.id != id
        {
            return false;
        }
        if let Some(from) = self.from_ms
            && entry.timestamp_ms < from
        {
            return false;
        }
        if let Some(to) = self.to_ms
            && entry.timestamp_ms > to
        {
            return false;
        }
        if let Some(capability) = &self.capability
            && &entry.capability != capability
        {
            return false;
        }
        true
    }
}

/// 应用级审计器：固定应用名、版本、身份、主机与会话，补齐每条记录的公共字段。
pub struct Auditor {
    app: String,
    app_version: String,
    actor: Actor,
    host: String,
    session: String,
    store: Store,
}

impl Auditor {
    /// 按环境变量与本机默认值构造审计器。
    pub fn from_env(app: impl Into<String>, app_version: impl Into<String>) -> Self {
        Self::builder(app, app_version).build()
    }

    /// 构造器：未显式设置的字段回退到环境变量与本机默认值。
    pub fn builder(app: impl Into<String>, app_version: impl Into<String>) -> AuditorBuilder {
        AuditorBuilder {
            app: app.into(),
            app_version: app_version.into(),
            actor: None,
            host: None,
            session: None,
            dir: None,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    /// 应用名。
    pub fn app(&self) -> &str {
        &self.app
    }

    /// 应用版本。
    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    /// 当前解析出的调用者身份。
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    /// 主机标识。
    pub fn host(&self) -> &str {
        &self.host
    }

    /// 会话关联 id。
    pub fn session(&self) -> &str {
        &self.session
    }

    /// 底层存储。
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// 查询审计记录。
    pub fn query(&self, filter: &Query) -> AuditResult<Vec<Entry>> {
        self.store.query(filter)
    }

    /// 查询审计记录并序列化为 JSON 数组文本。
    pub fn query_json(&self, filter: &Query) -> AuditResult<String> {
        let entries = self.query(filter)?;
        Ok(serde_json::to_string(&entries)?)
    }

    /// 开始一次调用；结束时用 [`Call::succeed`] / [`Call::fail`] / [`Call::reject`] 记录结果。
    pub fn call(&self, capability: impl Into<String>, mode: Mode) -> Call<'_> {
        Call {
            auditor: self,
            entry: Entry {
                schema: SCHEMA.into(),
                timestamp_ms: now_ms(),
                actor: self.actor.clone(),
                app: self.app.clone(),
                app_version: self.app_version.clone(),
                capability: capability.into(),
                params: Value::Object(Default::default()),
                mode,
                targets: Targets::default(),
                outcome: Outcome::Success,
                error_code: None,
                duration_ms: 0,
                host: self.host.clone(),
                session: self.session.clone(),
            },
            started: Instant::now(),
            finished: false,
        }
    }
}

/// 构造 [`Auditor`] 的可选字段覆盖。
pub struct AuditorBuilder {
    app: String,
    app_version: String,
    actor: Option<Actor>,
    host: Option<String>,
    session: Option<String>,
    dir: Option<PathBuf>,
    max_bytes: u64,
}

impl AuditorBuilder {
    /// 覆盖调用者身份。
    pub fn actor(mut self, actor: Actor) -> Self {
        self.actor = Some(actor);
        self
    }

    /// 覆盖主机标识。
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// 覆盖会话关联 id。
    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// 覆盖审计目录。
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// 覆盖单文件轮转阈值。
    pub fn max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// 构造审计器。
    pub fn build(self) -> Auditor {
        let actor = self.actor.unwrap_or_else(Actor::from_env);
        let host = self.host.unwrap_or_else(default_host);
        let session = self.session.unwrap_or_else(default_session);
        let dir = self.dir.unwrap_or_else(default_dir);
        let store = Store::new(dir, &self.app, self.max_bytes);
        Auditor { app: self.app, app_version: self.app_version, actor, host, session, store }
    }
}

/// 一次进行中的审计调用。
///
/// 未显式结束就析构时按 `failure` + `wae.audit::abandoned` 记录，避免漏记。
pub struct Call<'a> {
    auditor: &'a Auditor,
    entry: Entry,
    started: Instant,
    finished: bool,
}

impl Call<'_> {
    /// 设置业务参数，写入前自动脱敏。
    pub fn params(mut self, params: Value) -> Self {
        self.entry.params = sanitize_params(params);
        self
    }

    /// 设置 JSON 文本参数；解析失败时记录空对象。
    pub fn params_json(mut self, text: &str) -> Self {
        self.entry.params = serde_json::from_str(text).map(sanitize_params).unwrap_or_else(|_| Value::Object(Default::default()));
        self
    }

    /// 设置目标数量与摘要。
    pub fn targets(mut self, count: u64, summary: impl Into<String>) -> Self {
        self.entry.targets = Targets::new(count, summary);
        self
    }

    /// 记为成功。
    pub fn succeed(mut self) -> AuditResult<()> {
        self.finish(Outcome::Success, None)
    }

    /// 记为失败。
    pub fn fail(mut self, code: impl Into<String>) -> AuditResult<()> {
        self.finish(Outcome::Failure, Some(code.into()))
    }

    /// 记为被拒绝（授权或前置校验）。
    pub fn reject(mut self, code: impl Into<String>) -> AuditResult<()> {
        self.finish(Outcome::Rejected, Some(code.into()))
    }

    fn finish(&mut self, outcome: Outcome, error_code: Option<String>) -> AuditResult<()> {
        self.entry.outcome = outcome;
        self.entry.error_code = error_code;
        self.entry.duration_ms = self.started.elapsed().as_millis() as u64;
        self.finished = true;
        self.auditor.store.append(&self.entry)
    }
}

impl Drop for Call<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.entry.outcome = Outcome::Failure;
        self.entry.error_code = Some("wae.audit::abandoned".into());
        self.entry.duration_ms = self.started.elapsed().as_millis() as u64;
        self.finished = true;
        let _ = self.auditor.store.append(&self.entry);
    }
}

/// 当前 Unix 毫秒时间戳。
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 本机默认审计目录：`WAE_AUDIT_DIR`，否则为各平台应用数据目录下的 `wae/audit`。
pub fn default_dir() -> PathBuf {
    if let Some(dir) = env::var_os("WAE_AUDIT_DIR") {
        return PathBuf::from(dir);
    }
    platform_data_dir().unwrap_or_else(|| PathBuf::from(".")).join("wae").join("audit")
}

fn platform_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state")));
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    let base: Option<PathBuf> = None;
    base
}

fn default_host() -> String {
    env_text("WAE_HOST_ID")
        .or_else(|| env_text("COMPUTERNAME"))
        .or_else(|| env_text("HOSTNAME"))
        .or_else(|| env_text("HOST"))
        .unwrap_or_else(|| "unknown".into())
}

fn default_session() -> String {
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| format!("pid-{}-{}", process::id(), now_ms())).clone()
}

fn env_text(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn os_user() -> Option<String> {
    env_text("USERNAME").or_else(|| env_text("USER")).or_else(|| env_text("LOGNAME"))
}
