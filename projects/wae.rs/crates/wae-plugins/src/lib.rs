//! WAE 声明式插件宿主：清单加载、作用域收窄、候选归因与集中错误码。
//!
//! 插件只提交候选与处置请求，判定与执行永远在核心。本 crate 是五个 Open 应用
//! 共用的唯一加载器：不执行代码、不给插件文件句柄、不暴露删除入口。
//!
//! ```ignore
//! use wae::plugins::{CandidateSet, PackContext, load_pack};
//!
//! let context = PackContext::new(&["open-cleaner"], &["dev-artifacts"], &[]);
//! let pack = load_pack(std::path::Path::new("./plugins/cleaner-rules"), &context)?;
//! ```
#![deny(missing_docs)]

pub mod candidate;
pub mod error;
pub mod manifest;
pub mod scope;

pub use candidate::{
    Attribution, AttributionSource, CANDIDATE_SCHEMA, CandidateSet, CoreDecision, DecisionStatus, EvidenceFacts, EvidenceMap, EvidenceValue,
    FileType, PluginCandidate, ScopeViolation, recompute_evidence, verify_attribution, verify_evidence,
};
pub use error::{EXIT_DEGRADED, EXIT_FAILURE, EXIT_OK, ErrorCode, PluginError, PluginFailure};
pub use manifest::{
    BUILTIN_NAMESPACE, EVIDENCE_FIELDS, LoadedPack, MANIFEST_FILE, MatchCondition, PLUGIN_SCHEMA, PackContext, PluginKind, PluginManifest,
    PluginOrigin, PluginSource, Rule, RuleKind, RuleMatch, load_pack, validate_manifest,
};
pub use scope::{
    DEFAULT_SCOPE_MAX_BYTES, DEFAULT_SCOPE_MAX_DEPTH, ManifestScope, PathFacts, ResolvedRoot, ResolvedScope, ScanScope, ScopeResolution,
    resolve, validate_manifest_scope,
};

use std::time::Duration;

/// 每包默认超时（毫秒）。超时即整包失败并记 `plugin_timeout`。
pub const DEFAULT_PLUGIN_TIMEOUT_MS: u64 = 5_000;

/// 超时下限（毫秒）。
pub const PLUGIN_TIMEOUT_MIN_MS: u64 = 100;

/// 超时上限（毫秒）。
pub const PLUGIN_TIMEOUT_MAX_MS: u64 = 30_000;

/// 每包默认候选上限。超限即整包失败并记 `plugin_candidate_limit`。
pub const DEFAULT_PLUGIN_MAX_CANDIDATES: usize = 10_000;

/// 候选上限的上界。
pub const PLUGIN_MAX_CANDIDATES_LIMIT: usize = 100_000;

/// 单包预算。超限进入 `pluginFailures[]`，不静默截断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginBudget {
    /// 时间预算。
    pub timeout_ms: u64,
    /// 候选上限。
    pub max_candidates: usize,
}

impl PluginBudget {
    /// 构造预算。
    pub const fn new(timeout_ms: u64, max_candidates: usize) -> Self {
        Self { timeout_ms, max_candidates }
    }

    /// 是否已超过时间预算。调用方按规则顺序累计耗时，不依赖后台线程。
    pub fn timed_out(&self, elapsed: Duration) -> bool {
        elapsed.as_millis() as u64 > self.timeout_ms
    }
}

impl Default for PluginBudget {
    fn default() -> Self {
        Self { timeout_ms: DEFAULT_PLUGIN_TIMEOUT_MS, max_candidates: DEFAULT_PLUGIN_MAX_CANDIDATES }
    }
}
