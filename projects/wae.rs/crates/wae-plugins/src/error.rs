//! 插件错误码与退出码的唯一集中定义处。
//!
//! 其它模块只构造 [`PluginError`]，不再各自维护字符串或退出码。新增错误码时
//! 必须同时补 [`ErrorCode::as_str`] 与 [`ErrorCode::exit_code`]。

use serde::Serialize;
use thiserror::Error;

/// 扫描正常结束。
pub const EXIT_OK: i32 = 0;

/// 致命错误：配置、清单、来源或 IO 失败，扫描不可信。
pub const EXIT_FAILURE: i32 = 1;

/// 降级扫描：至少一个插件失败或超预算，但其它来源仍然完成。
///
/// 数值待 `failOnPluginError` 口径确认，先集中在此处，禁止下游写魔法数字。
pub const EXIT_DEGRADED: i32 = 2;

/// 稳定错误码。序列化为本文合同的 `plugin_*` 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    /// 字段缺失、未知字段、类型错误或重复键。
    #[serde(rename = "plugin_manifest_invalid")]
    ManifestInvalid,
    /// `schema` 缺失或不是 `wae.plugin/v0`。
    #[serde(rename = "plugin_version_unsupported")]
    VersionUnsupported,
    /// `kind` 不是 `rules`。
    #[serde(rename = "plugin_kind_unsupported")]
    KindUnsupported,
    /// 作用域字段本身非法。
    #[serde(rename = "plugin_scope_invalid")]
    ScopeInvalid,
    /// 单个候选越界；只丢弃该候选。
    #[serde(rename = "plugin_scope_violation")]
    ScopeViolation,
    /// 规则 id 重复或与内置命名空间冲突。
    #[serde(rename = "plugin_rule_conflict")]
    RuleConflict,
    /// `domain` 不在应用声明列表。
    #[serde(rename = "plugin_domain_unsupported")]
    DomainUnsupported,
    /// `sha256` 与配置 pin 或清单不一致。
    #[serde(rename = "plugin_source_mismatch")]
    SourceMismatch,
    /// 核心复算与插件声称不符；只跳过该候选。
    #[serde(rename = "plugin_evidence_mismatch")]
    EvidenceMismatch,
    /// 超过候选预算；整包失败。
    #[serde(rename = "plugin_candidate_limit")]
    CandidateLimit,
    /// 超过时间预算；整包失败。
    #[serde(rename = "plugin_timeout")]
    Timeout,
    /// 读取或解析清单失败。
    #[serde(rename = "plugin_io_failed")]
    IoFailed,
}

impl ErrorCode {
    /// 稳定字符串，与序列化值一致。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManifestInvalid => "plugin_manifest_invalid",
            Self::VersionUnsupported => "plugin_version_unsupported",
            Self::KindUnsupported => "plugin_kind_unsupported",
            Self::ScopeInvalid => "plugin_scope_invalid",
            Self::ScopeViolation => "plugin_scope_violation",
            Self::RuleConflict => "plugin_rule_conflict",
            Self::DomainUnsupported => "plugin_domain_unsupported",
            Self::SourceMismatch => "plugin_source_mismatch",
            Self::EvidenceMismatch => "plugin_evidence_mismatch",
            Self::CandidateLimit => "plugin_candidate_limit",
            Self::Timeout => "plugin_timeout",
            Self::IoFailed => "plugin_io_failed",
        }
    }

    /// 该错误对整次扫描的退出码影响。
    ///
    /// 候选级拒绝（越界、证据不符）不改变退出码；包级失败与预算耗尽进入降级。
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::ScopeViolation | Self::EvidenceMismatch => EXIT_OK,
            Self::CandidateLimit | Self::Timeout => EXIT_DEGRADED,
            Self::ManifestInvalid
            | Self::VersionUnsupported
            | Self::KindUnsupported
            | Self::ScopeInvalid
            | Self::RuleConflict
            | Self::DomainUnsupported
            | Self::SourceMismatch
            | Self::IoFailed => EXIT_FAILURE,
        }
    }

    /// 包级失败：该插件本次不参与扫描。
    pub const fn is_pack_fatal(self) -> bool {
        matches!(
            self,
            Self::ManifestInvalid
                | Self::VersionUnsupported
                | Self::KindUnsupported
                | Self::ScopeInvalid
                | Self::RuleConflict
                | Self::DomainUnsupported
                | Self::SourceMismatch
                | Self::CandidateLimit
                | Self::Timeout
                | Self::IoFailed
        )
    }
}

/// 插件错误：稳定 code + 稳定 reason + 人类可读 message。
#[derive(Debug, Clone, Error, Serialize)]
#[error("{}: {} ({})", code.as_str(), message, reason)]
pub struct PluginError {
    /// 稳定错误码。
    pub code: ErrorCode,
    /// 稳定 reason，供机读与测试断言。
    pub reason: String,
    /// 人类可读说明，不放内部路径清单。
    pub message: String,
    /// 出错插件 id；加载阶段尚不可知时为空。
    pub plugin_id: Option<String>,
}

impl PluginError {
    /// 构造一个插件错误。
    pub fn new(code: ErrorCode, reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code, reason: reason.into(), message: message.into(), plugin_id: None }
    }

    /// 补上插件 id，便于结果归因。
    pub fn with_plugin(mut self, plugin_id: impl Into<String>) -> Self {
        self.plugin_id = Some(plugin_id.into());
        self
    }

    /// 该错误是否应拒绝整包。
    pub const fn is_pack_fatal(&self) -> bool {
        self.code.is_pack_fatal()
    }
}

/// 结构化失败条目；降级扫描必须带至少一个，不得折叠为空候选。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginFailure {
    /// 插件 id；清单尚未解析成功时使用目录名。
    pub plugin_id: String,
    /// 稳定错误码。
    pub code: &'static str,
    /// 稳定 reason。
    pub reason: String,
    /// 人类可读说明。
    pub message: String,
    /// 该包本次耗时。
    pub elapsed_ms: u64,
}

impl PluginFailure {
    /// 从错误构造失败条目。
    pub fn from_error(plugin_id: impl Into<String>, error: &PluginError, elapsed_ms: u64) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            code: error.code.as_str(),
            reason: error.reason.clone(),
            message: error.message.clone(),
            elapsed_ms,
        }
    }
}
