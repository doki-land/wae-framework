//! 候选归因：`wae.plugin-candidate/v0`、证据复算与逐条越界丢弃。
//!
//! 插件只提交候选与**声称**事实；核心在同一快照上复算证据。两者不一致时该候选
//! 跳过并记 `plugin_evidence_mismatch`。同一路径保留全部归因，计划项只有一条。

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer},
    ser::Serializer,
};

use crate::{
    error::{ErrorCode, PluginError},
    manifest::{Disposition, PluginManifest, Rule},
    scope::{PathFacts, ResolvedScope},
};

/// 候选归因输出 schema。
pub const CANDIDATE_SCHEMA: &str = "wae.plugin-candidate/v0";

/// 单条候选的证据字段集合，键只能是 `evidence` 白名单字段。
pub type EvidenceMap = BTreeMap<String, EvidenceValue>;

/// 证据值。只允许 null、布尔、整数与字符串，禁止嵌套结构。
#[derive(Debug, Clone, PartialEq)]
pub enum EvidenceValue {
    /// JSON `null`：领域未提供该字段。
    Null,
    /// 布尔事实，如 `symlink`。
    Bool(bool),
    /// 计数或天数。
    Integer(i64),
    /// 路径、类型与 id。
    Text(String),
}

impl Serialize for EvidenceValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Text(value) => serializer.serialize_str(value),
        }
    }
}

impl<'de> Deserialize<'de> for EvidenceValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(Self::Null),
            serde_json::Value::Bool(value) => Ok(Self::Bool(value)),
            serde_json::Value::Number(number) => {
                number.as_i64().map(Self::Integer).ok_or_else(|| de::Error::custom("evidence numbers must be integers"))
            }
            serde_json::Value::String(value) => Ok(Self::Text(value)),
            _ => Err(de::Error::custom("evidence values must be null, bool, integer or string")),
        }
    }
}

/// 核心复算的文件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// 普通文件。
    File,
    /// 目录。
    Directory,
}

impl FileType {
    /// 稳定字符串，与 matcher `type` 取值一致。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

/// 核心复算的事实。插件不能提供这些值，只能声明需要哪些字段。
#[derive(Debug, Clone)]
pub struct EvidenceFacts {
    /// 规范化后的候选路径。
    pub path: PathBuf,
    /// 锚点文件名；规则未声明锚点时为空。
    pub anchor: Option<String>,
    /// 统计体积。
    pub size_bytes: u64,
    /// 距今天数，基于 mtime。
    pub mtime_days: u64,
    /// 文件类型。
    pub file_type: FileType,
    /// 是否符号链接。
    pub symlink: bool,
    /// 判定所用领域 id。
    pub domain: String,
}

/// 归因来源。`origin` 取值 `local` / `registry` / `builtin`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttributionSource {
    /// 来源类型。
    pub origin: String,
    /// `plugin.json` 的 sha256；内置来源为空。
    pub sha256: String,
}

impl AttributionSource {
    /// 内置规则的来源标注。
    pub fn builtin() -> Self {
        Self { origin: "builtin".to_owned(), sha256: String::new() }
    }
}

/// 一条候选归因：谁声明了它、声明了什么、请求了什么处置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attribution {
    /// 插件 id；内置规则为 `builtin`。
    pub plugin_id: String,
    /// 插件版本。
    pub plugin_version: String,
    /// 规则 id。
    pub rule_id: String,
    /// 规则版本。
    pub rule_version: String,
    /// 来源。
    pub source: AttributionSource,
    /// 匹配阶段采集的证据；核心会逐字段复算。
    #[serde(default)]
    pub evidence: EvidenceMap,
    /// 插件请求的处置，不是核心决定。
    pub requested_disposition: Disposition,
}

impl Attribution {
    /// 从清单规则与匹配阶段证据构造归因。
    pub fn from_rule(manifest: &PluginManifest, rule: &Rule, evidence: EvidenceMap) -> Self {
        Self {
            plugin_id: manifest.id.clone(),
            plugin_version: manifest.version.clone(),
            rule_id: rule.id.clone(),
            rule_version: rule.version.clone(),
            source: AttributionSource { origin: manifest.source.origin.as_str().to_owned(), sha256: manifest.source.sha256.clone() },
            evidence,
            requested_disposition: rule.disposition.clone(),
        }
    }
}

/// 核心决定状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionStatus {
    /// 进入计划，等待显式确认。
    Planned,
    /// 跳过；必须带稳定 reason。
    Skipped,
    /// 执行失败；必须带稳定 reason。
    Failed,
}

/// 核心决定。`skipped` / `failed` 必须带稳定 `reason`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreDecision {
    /// 状态。
    pub status: DecisionStatus,
    /// 实际处置；只报告或跳过时可为空。
    pub disposition: Option<String>,
    /// 稳定原因。
    pub reason: Option<String>,
}

impl CoreDecision {
    /// 进入计划。
    pub fn planned(disposition: impl Into<String>) -> Self {
        Self { status: DecisionStatus::Planned, disposition: Some(disposition.into()), reason: None }
    }

    /// 跳过并给出稳定原因。
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self { status: DecisionStatus::Skipped, disposition: None, reason: Some(reason.into()) }
    }

    /// 执行失败并给出稳定原因。
    pub fn failed(reason: impl Into<String>) -> Self {
        Self { status: DecisionStatus::Failed, disposition: None, reason: Some(reason.into()) }
    }
}

/// 归因输出中的一条候选；同路径只有一条计划项。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginCandidate {
    /// 固定为 [`CANDIDATE_SCHEMA`]。
    pub schema: String,
    /// 候选路径。
    pub path: PathBuf,
    /// 全部归因，内置与插件同时命中时全部保留。
    pub attributions: Vec<Attribution>,
    /// 核心决定。
    pub core_decision: CoreDecision,
}

impl PluginCandidate {
    /// 构造一条候选输出。
    pub fn new(path: impl Into<PathBuf>, core_decision: CoreDecision) -> Self {
        Self { schema: CANDIDATE_SCHEMA.to_owned(), path: path.into(), attributions: Vec::new(), core_decision }
    }
}

/// 一条越界候选的结构化记录；扫描继续，候选丢弃。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeViolation {
    /// 插件 id。
    pub plugin_id: String,
    /// 规则 id。
    pub rule_id: String,
    /// 插件提交的路径。
    pub declared_path: String,
    /// 核心解析后的实际路径；本层未额外解析时与声明一致。
    pub actual_path: String,
    /// 稳定 reason，如 `out_of_root`、`exceeds_depth`。
    pub reason: String,
}

impl ScopeViolation {
    /// 从作用域判定错误构造记录。
    pub fn from_error(attribution: &Attribution, declared: &Path, error: &PluginError) -> Self {
        let path = declared.to_string_lossy().into_owned();
        Self {
            plugin_id: attribution.plugin_id.clone(),
            rule_id: attribution.rule_id.clone(),
            declared_path: path.clone(),
            actual_path: path,
            reason: error.reason.clone(),
        }
    }
}

/// 按证据字段名在核心事实里取值。字段顺序即输出顺序。
pub fn recompute_evidence(fields: &[String], facts: &EvidenceFacts, attribution: &Attribution) -> Result<EvidenceMap, PluginError> {
    let mut map = EvidenceMap::new();
    for field in fields {
        let value = match field.as_str() {
            "path" => EvidenceValue::Text(facts.path.to_string_lossy().into_owned()),
            "anchor" => facts.anchor.as_ref().map_or(EvidenceValue::Null, |anchor| EvidenceValue::Text(anchor.clone())),
            "sizeBytes" => EvidenceValue::Integer(i64::try_from(facts.size_bytes).unwrap_or(i64::MAX)),
            "mtimeDays" => EvidenceValue::Integer(i64::try_from(facts.mtime_days).unwrap_or(i64::MAX)),
            "fileType" => EvidenceValue::Text(facts.file_type.as_str().to_owned()),
            "symlink" => EvidenceValue::Bool(facts.symlink),
            "domain" => EvidenceValue::Text(facts.domain.clone()),
            "ruleId" => EvidenceValue::Text(attribution.rule_id.clone()),
            "pluginId" => EvidenceValue::Text(attribution.plugin_id.clone()),
            other => {
                return Err(evidence_mismatch("unsupported_evidence", format!("evidence field `{other}` is outside the whitelist")));
            }
        };
        map.insert(field.clone(), value);
    }
    Ok(map)
}

/// 逐字段比较声称与复算结果；任一字段不同即 `plugin_evidence_mismatch`。
pub fn verify_evidence(claims: &EvidenceMap, recomputed: &EvidenceMap) -> Result<(), PluginError> {
    for (field, claimed) in claims {
        let Some(expected) = recomputed.get(field)
        else {
            return Err(evidence_mismatch("missing_fact", format!("core did not recompute evidence field `{field}`")));
        };
        if claimed != expected {
            return Err(evidence_mismatch(mismatch_reason(field), format!("evidence field `{field}` differs from the recomputed fact")));
        }
    }
    Ok(())
}

/// 一步完成「按归因声明的字段复算并比对」，返回核心证据。
pub fn verify_attribution(attribution: &Attribution, facts: &EvidenceFacts) -> Result<EvidenceMap, PluginError> {
    let fields: Vec<String> = attribution.evidence.keys().cloned().collect();
    let recomputed = recompute_evidence(&fields, facts, attribution)?;
    verify_evidence(&attribution.evidence, &recomputed)?;
    Ok(recomputed)
}

fn mismatch_reason(field: &str) -> &'static str {
    match field {
        "path" => "path_mismatch",
        "anchor" => "anchor_mismatch",
        "sizeBytes" => "size_mismatch",
        "mtimeDays" => "mtime_mismatch",
        "fileType" => "file_type_mismatch",
        "symlink" => "symlink_mismatch",
        "domain" => "domain_mismatch",
        "ruleId" => "rule_id_mismatch",
        "pluginId" => "plugin_id_mismatch",
        _ => "unsupported_evidence",
    }
}

fn evidence_mismatch(reason: &str, message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorCode::EvidenceMismatch, reason, message)
}

/// 单包内的候选聚合。超预算即整包失败，不做静默截断。
#[derive(Debug, Clone)]
pub struct CandidateSet {
    limit: usize,
    entries: BTreeMap<PathBuf, Vec<Attribution>>,
}

impl CandidateSet {
    /// 以候选上限构造；默认值见 crate 常量。
    pub fn new(limit: usize) -> Self {
        Self { limit, entries: BTreeMap::new() }
    }

    /// 已聚合的不同路径数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有任何候选。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 追加一条归因；同路径保留全部归因，不新增计划项。
    ///
    /// 超过上限返回 `plugin_candidate_limit`，调用方应丢弃整包结果。
    pub fn insert(&mut self, path: impl Into<PathBuf>, attribution: Attribution) -> Result<(), PluginError> {
        let path = path.into();
        if let Some(existing) = self.entries.get_mut(&path) {
            existing.push(attribution);
            return Ok(());
        }
        if self.entries.len() >= self.limit {
            return Err(PluginError::new(
                ErrorCode::CandidateLimit,
                "candidate_limit",
                format!("pack exceeded the candidate budget of {}", self.limit),
            ));
        }
        self.entries.insert(path, vec![attribution]);
        Ok(())
    }

    /// 先做作用域判定，再入集合。越界只丢弃该候选并记 [`ScopeViolation`]。
    pub fn insert_scoped(
        &mut self,
        scope: &ResolvedScope,
        path: impl Into<PathBuf>,
        facts: PathFacts,
        attribution: Attribution,
        violations: &mut Vec<ScopeViolation>,
    ) -> Result<(), PluginError> {
        let path = path.into();
        match scope.accepts(&path, facts) {
            Ok(()) => self.insert(path, attribution),
            Err(error) => {
                violations.push(ScopeViolation::from_error(&attribution, &path, &error));
                Ok(())
            }
        }
    }

    /// 把聚合结果交给核心判定，输出归因结构。
    pub fn into_candidates(self, mut decide: impl FnMut(&Path) -> CoreDecision) -> Vec<PluginCandidate> {
        self.entries
            .into_iter()
            .map(|(path, attributions)| {
                let core_decision = decide(&path);
                PluginCandidate { schema: CANDIDATE_SCHEMA.to_owned(), path, attributions, core_decision }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::scope::ManifestScope;

    fn rule() -> Rule {
        Rule {
            id: "dev-artifact-node-modules".to_owned(),
            version: "1".to_owned(),
            domain: "dev-artifacts".to_owned(),
            kind: crate::manifest::RuleKind::Cleanup,
            matching: crate::manifest::RuleMatch {
                all: vec![crate::manifest::MatchCondition { entry_type: Some("directory".to_owned()), ..Default::default() }],
            },
            evidence: vec!["anchor".to_owned(), "sizeBytes".to_owned()],
            disposition: Disposition { request: "trash".to_owned(), requires: Vec::new(), on_failure: "skip".to_owned() },
            default_selected: false,
        }
    }

    fn manifest() -> PluginManifest {
        PluginManifest {
            schema: Some("wae.plugin/v0".to_owned()),
            kind: "rules".to_owned(),
            id: "com.example.cleaner-rules".to_owned(),
            name: "Example Cleaner Rules".to_owned(),
            version: "1.0.0".to_owned(),
            source: crate::manifest::PluginSource {
                origin: crate::manifest::PluginOrigin::Local,
                uri: "./plugins/cleaner-rules".to_owned(),
                sha256: "0".repeat(64),
            },
            applies_to: vec!["open-cleaner".to_owned()],
            scope: ManifestScope { roots: vec![".".to_owned()], excludes: Vec::new(), max_depth: 6, max_bytes: 1_048_576 },
            rules: vec![rule()],
        }
    }

    fn attribution(evidence: EvidenceMap) -> Attribution {
        let manifest = manifest();
        Attribution::from_rule(&manifest, &rule(), evidence)
    }

    fn facts() -> EvidenceFacts {
        EvidenceFacts {
            path: PathBuf::from("C:/work/app/node_modules"),
            anchor: Some("package.json".to_owned()),
            size_bytes: 1_048_576,
            mtime_days: 7,
            file_type: FileType::Directory,
            symlink: false,
            domain: "dev-artifacts".to_owned(),
        }
    }

    #[test]
    fn evidence_round_trips_through_json() {
        let mut claims = EvidenceMap::new();
        claims.insert("path".to_owned(), EvidenceValue::Text("C:/work/a".to_owned()));
        claims.insert("sizeBytes".to_owned(), EvidenceValue::Integer(42));
        claims.insert("symlink".to_owned(), EvidenceValue::Bool(false));
        claims.insert("anchor".to_owned(), EvidenceValue::Null);
        let json = serde_json::to_string(&claims).unwrap();
        let decoded: EvidenceMap = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, claims);
        assert!(json.contains("\"anchor\":null"));
    }

    #[test]
    fn matching_evidence_passes() {
        let recomputed =
            recompute_evidence(&["anchor".to_owned(), "sizeBytes".to_owned()], &facts(), &attribution(EvidenceMap::new())).unwrap();
        assert_eq!(recomputed["anchor"], EvidenceValue::Text("package.json".to_owned()));
        assert_eq!(recomputed["sizeBytes"], EvidenceValue::Integer(1_048_576));
        verify_evidence(&recomputed, &recomputed).unwrap();
    }

    #[test]
    fn stated_size_that_differs_is_rejected() {
        let mut claims = EvidenceMap::new();
        claims.insert("sizeBytes".to_owned(), EvidenceValue::Integer(7));
        let error = verify_attribution(&attribution(claims), &facts()).unwrap_err();
        assert_eq!(error.code, ErrorCode::EvidenceMismatch);
        assert_eq!(error.reason, "size_mismatch");
    }

    #[test]
    fn unsupported_evidence_field_is_rejected() {
        let error = recompute_evidence(&["contentHash".to_owned()], &facts(), &attribution(EvidenceMap::new())).unwrap_err();
        assert_eq!(error.reason, "unsupported_evidence");
    }

    #[test]
    fn candidate_set_merges_attributions_per_path() {
        let mut set = CandidateSet::new(2);
        let one = attribution(EvidenceMap::new());
        let two = attribution(EvidenceMap::new());
        set.insert("C:/work/node_modules", one).unwrap();
        set.insert("C:/work/node_modules", two).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.into_candidates(|_| CoreDecision::planned("trash"))[0].attributions.len(), 2);
    }

    #[test]
    fn candidate_set_reports_the_limit_instead_of_truncating() {
        let mut set = CandidateSet::new(1);
        set.insert("C:/work/a", attribution(EvidenceMap::new())).unwrap();
        let error = set.insert("C:/work/b", attribution(EvidenceMap::new())).unwrap_err();
        assert_eq!(error.code, ErrorCode::CandidateLimit);
        assert_eq!(error.reason, "candidate_limit");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn out_of_scope_candidates_are_dropped_and_recorded() {
        let scope = ResolvedScope {
            roots: vec![crate::scope::ResolvedRoot { path: PathBuf::from("C:/work/src"), excludes: Vec::new() }],
            max_depth: 4,
            max_bytes: 1024,
        };
        let mut set = CandidateSet::new(4);
        let mut violations = Vec::new();
        set.insert_scoped(
            &scope,
            "C:/work/other/file",
            PathFacts { size_bytes: 1, symlink: false },
            attribution(EvidenceMap::new()),
            &mut violations,
        )
        .unwrap();
        assert!(set.is_empty());
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].reason, "out_of_root");
        let value = serde_json::to_value(&violations[0]).unwrap();
        assert_eq!(value["pluginId"], json!("com.example.cleaner-rules"));
        assert_eq!(value["reason"], json!("out_of_root"));
    }

    #[test]
    fn candidate_output_uses_the_documented_schema() {
        let candidate = PluginCandidate {
            schema: CANDIDATE_SCHEMA.to_owned(),
            path: PathBuf::from("C:/work/app/node_modules"),
            attributions: vec![attribution(EvidenceMap::new())],
            core_decision: CoreDecision::skipped("plugin_evidence_mismatch"),
        };
        let value = serde_json::to_value(&candidate).unwrap();
        assert_eq!(value["schema"], json!("wae.plugin-candidate/v0"));
        assert_eq!(value["coreDecision"]["status"], json!("skipped"));
        assert_eq!(value["attributions"][0]["source"]["origin"], json!("local"));
        assert_eq!(value["coreDecision"]["reason"], json!("plugin_evidence_mismatch"));
    }
}
