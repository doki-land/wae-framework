//! 清单解析与整包原子拒绝。
//!
//! 一个插件包只有一个入口 `plugin.json`，解析严格：非 UTF-8、重复键、未知字段、
//! 未知类型、逃逸作用域与规则冲突一律拒绝整包。不存在「部分规则可用」的中间状态。

use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{ErrorCode, PluginError},
    scope::{ManifestScope, validate_manifest_scope},
};

/// 当前插件清单 schema，合同命名空间固定 `/v0`。
pub const PLUGIN_SCHEMA: &str = "wae.plugin/v0";

/// 包目录内唯一被读取的入口文件名。
pub const MANIFEST_FILE: &str = "plugin.json";

/// 内置命名空间；插件包 id 与规则 id 不得占用。
pub const BUILTIN_NAMESPACE: &str = "builtin";

/// 证据字段白名单（§8.4）。插件只能引用这些字段名，不能自带字段值。
pub const EVIDENCE_FIELDS: [&str; 9] = ["path", "anchor", "sizeBytes", "mtimeDays", "fileType", "symlink", "domain", "ruleId", "pluginId"];

/// 枚举型 matcher 的条目上限。
const MAX_ENUM_ENTRIES: usize = 64;

/// 插件来源类型。首版只有本地目录与受控 registry 标识，不联网。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginOrigin {
    /// 本地目录。
    Local,
    /// 受控 registry 标识，只作来源标注。
    Registry,
}

impl PluginOrigin {
    /// 稳定字符串，用于归因输出。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Registry => "registry",
        }
    }
}

/// 插件种类。首版只接受 `rules`；`code` 只保留拒绝路径，不设计成可选后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    /// 声明式规则包。
    Rules,
    /// 代码级扩展；出现即 `plugin_kind_unsupported`。
    Code,
}

impl PluginKind {
    /// 解析清单里的 `kind` 字符串；未知取值同样是 `plugin_kind_unsupported`。
    pub fn parse(value: &str) -> Result<Self, PluginError> {
        match value {
            "rules" => Ok(Self::Rules),
            "code" => Err(kind_unsupported(value)),
            _ => Err(kind_unsupported(value)),
        }
    }
}

fn kind_unsupported(value: &str) -> PluginError {
    PluginError::new(ErrorCode::KindUnsupported, "kind_unsupported", format!("plugin kind `{value}` is not supported, only `rules`"))
}

/// 清单来源块。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSource {
    /// 来源类型。
    pub origin: PluginOrigin,
    /// 相对项目根的目录，或受控 registry 标识。
    pub uri: String,
    /// `plugin.json` 的 sha256；与配置 pin 不一致即加载失败。
    pub sha256: String,
}

/// 规则种类。`cleanup` 才允许非 `report` 处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleKind {
    /// 只分析，不产生执行项。
    Analysis,
    /// 可请求清理。
    Cleanup,
}

/// 处置**请求**，不是决定。闭集取值仍在确认中，这里只做形状校验。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Disposition {
    /// 请求的处置动作。
    pub request: String,
    /// 插件认为需要满足的前置条件名；不改变核心判定。
    #[serde(default)]
    pub requires: Vec<String>,
    /// 条件不满足时插件期望的行为；不改变核心判定。
    #[serde(default = "default_on_failure")]
    pub on_failure: String,
}

fn default_on_failure() -> String {
    "skip".to_owned()
}

/// 规则的匹配块，只允许 [`MatchCondition`] 的固定 matcher。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleMatch {
    /// 必须全部满足的条件，至少一条。
    pub all: Vec<MatchCondition>,
}

/// 单个 matcher 条件。字段闭集，每个条件必须恰好声明一个。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatchCondition {
    /// `file` 或 `directory`。
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    /// 精确文件名。
    pub name_equals: Option<String>,
    /// 文件名枚举，最多 64 项。
    pub name_in: Option<Vec<String>>,
    /// 规范化路径后缀枚举，最多 64 项。
    pub path_suffix_in: Option<Vec<String>>,
    /// 同目录存在指定文件名。
    pub sibling_exists: Option<Vec<String>>,
    /// 体积下限。
    pub min_bytes: Option<u64>,
    /// 体积上限。
    pub max_bytes: Option<u64>,
    /// 距今最少天数。
    pub min_age_days: Option<u32>,
}

impl MatchCondition {
    /// 校验条件形状与取值，返回实际使用的 matcher 名。
    fn validate(&self, rule_id: &str) -> Result<&'static str, PluginError> {
        let mut names = Vec::new();
        if self.entry_type.is_some() {
            names.push("type");
        }
        if self.name_equals.is_some() {
            names.push("nameEquals");
        }
        if self.name_in.is_some() {
            names.push("nameIn");
        }
        if self.path_suffix_in.is_some() {
            names.push("pathSuffixIn");
        }
        if self.sibling_exists.is_some() {
            names.push("siblingExists");
        }
        if self.min_bytes.is_some() {
            names.push("minBytes");
        }
        if self.max_bytes.is_some() {
            names.push("maxBytes");
        }
        if self.min_age_days.is_some() {
            names.push("minAgeDays");
        }
        let Some(name) = names.first().copied()
        else {
            return Err(manifest_invalid(
                "empty_match_condition",
                format!("rule `{rule_id}` declares a matcher condition without any matcher"),
            ));
        };
        if names.len() > 1 {
            return Err(manifest_invalid(
                "match_condition_conflict",
                format!("rule `{rule_id}` declares more than one matcher in one condition"),
            ));
        }

        match name {
            "type" => {
                let value = self.entry_type.as_deref().unwrap_or_default();
                if !matches!(value, "file" | "directory") {
                    return Err(manifest_invalid(
                        "invalid_match_value",
                        format!("rule `{rule_id}` declares `type` other than `file` or `directory`"),
                    ));
                }
            }
            "nameEquals" => check_scalar(self.name_equals.as_deref(), rule_id, "nameEquals")?,
            "nameIn" => check_enum(self.name_in.as_deref(), rule_id, "nameIn")?,
            "pathSuffixIn" => check_enum(self.path_suffix_in.as_deref(), rule_id, "pathSuffixIn")?,
            "siblingExists" => check_enum(self.sibling_exists.as_deref(), rule_id, "siblingExists")?,
            "minBytes" | "maxBytes" | "minAgeDays" => {}
            _ => {}
        }
        Ok(name)
    }
}

fn check_scalar(value: Option<&str>, rule_id: &str, matcher: &str) -> Result<(), PluginError> {
    if value.is_some_and(|value| value.is_empty()) {
        return Err(manifest_invalid("empty_match_value", format!("rule `{rule_id}` declares an empty `{matcher}`")));
    }
    Ok(())
}

fn check_enum(values: Option<&[String]>, rule_id: &str, matcher: &str) -> Result<(), PluginError> {
    let Some(values) = values
    else {
        return Ok(());
    };
    if values.len() > MAX_ENUM_ENTRIES {
        return Err(manifest_invalid(
            "too_many_match_values",
            format!("rule `{rule_id}` declares more than {MAX_ENUM_ENTRIES} `{matcher}` entries"),
        ));
    }
    if values.iter().any(|value| value.is_empty()) {
        return Err(manifest_invalid("empty_match_value", format!("rule `{rule_id}` declares an empty `{matcher}` entry")));
    }
    Ok(())
}

/// 一条规则：只声明匹配条件、可选证据字段与处置请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    /// 包内唯一；跨包与内置规则全局唯一。
    pub id: String,
    /// 规则版本，变化必须递增。
    pub version: String,
    /// 应用声明的领域 id。
    pub domain: String,
    /// 规则种类。
    pub kind: RuleKind,
    /// 匹配条件。
    #[serde(rename = "match")]
    pub matching: RuleMatch,
    /// 只允许 [`EVIDENCE_FIELDS`] 的字段名。
    pub evidence: Vec<String>,
    /// 处置请求。
    pub disposition: Disposition,
    /// 默认勾选，仅影响预览，不影响安全判定。
    #[serde(default)]
    pub default_selected: bool,
}

/// 插件清单。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    /// 合同版本；缺失按 `plugin_version_unsupported` 拒绝。
    #[serde(default)]
    pub schema: Option<String>,
    /// 包种类；首版只接受 `rules`。
    pub kind: String,
    /// 反向域名风格的全局唯一 id。
    pub id: String,
    /// 人类可读名称，不参与判定。
    pub name: String,
    /// semver，只用于归因与来源核对。
    pub version: String,
    /// 来源块。
    pub source: PluginSource,
    /// 应用 id 列表，空数组拒绝。
    pub applies_to: Vec<String>,
    /// 声明作用域。
    pub scope: ManifestScope,
    /// 规则数组，至少一条。
    pub rules: Vec<Rule>,
}

/// 领域侧提供的绑定信息，用于校验 `appliesTo`、`domain` 与内置规则冲突。
#[derive(Debug, Clone, Copy)]
pub struct PackContext<'a> {
    /// 应用 id 集合。
    pub apps: &'a [&'a str],
    /// 应用声明的领域 id 集合。
    pub domains: &'a [&'a str],
    /// 内置规则 id 集合。
    pub builtin_rule_ids: &'a [&'a str],
}

impl<'a> PackContext<'a> {
    /// 构造绑定上下文。
    pub const fn new(apps: &'a [&'a str], domains: &'a [&'a str], builtin_rule_ids: &'a [&'a str]) -> Self {
        Self { apps, domains, builtin_rule_ids }
    }
}

/// 成功加载的包：清单已通过全部校验，可进入作用域解析。
#[derive(Debug, Clone)]
pub struct LoadedPack {
    /// 已校验的清单。
    pub manifest: PluginManifest,
    /// 规范化后的包目录。
    pub dir: PathBuf,
    /// `plugin.json` 原始字节；供来源 pin 核对使用。
    pub manifest_bytes: Vec<u8>,
}

/// 读取并校验一个插件包；任何失败都拒绝整包。
pub fn load_pack(dir: &Path, context: &PackContext<'_>) -> Result<LoadedPack, PluginError> {
    let metadata = fs::symlink_metadata(dir).map_err(|error| io_failed("pack_unreadable", format!("cannot read pack directory: {error}")))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(manifest_invalid("symlink_pack", "pack directory must not be a symlink or junction"));
    }
    if !file_type.is_dir() {
        return Err(manifest_invalid("pack_not_directory", "pack path must be a directory"));
    }

    let manifest_path = dir.join(MANIFEST_FILE);
    let manifest_meta = fs::symlink_metadata(&manifest_path)
        .map_err(|error| io_failed("manifest_missing", format!("cannot read `{MANIFEST_FILE}`: {error}")))?;
    let manifest_type = manifest_meta.file_type();
    if manifest_type.is_symlink() {
        return Err(manifest_invalid("symlink_manifest", format!("`{MANIFEST_FILE}` must not be a symlink")));
    }
    if !manifest_type.is_file() {
        return Err(manifest_invalid("manifest_not_file", format!("`{MANIFEST_FILE}` must be a regular file")));
    }

    let bytes =
        fs::read(&manifest_path).map_err(|error| io_failed("manifest_unreadable", format!("cannot read `{MANIFEST_FILE}`: {error}")))?;
    let manifest = parse_manifest(&bytes)?;
    validate_manifest(&manifest, context)?;

    let dir = fs::canonicalize(dir).map_err(|error| io_failed("pack_unresolved", format!("cannot resolve pack directory: {error}")))?;
    Ok(LoadedPack { manifest, dir, manifest_bytes: bytes })
}

fn parse_manifest(bytes: &[u8]) -> Result<PluginManifest, PluginError> {
    let text = std::str::from_utf8(bytes).map_err(|_| manifest_invalid("non_utf8", format!("`{MANIFEST_FILE}` must be UTF-8")))?.to_owned();
    scan_duplicate_keys(&text)?;
    serde_json::from_str(&text).map_err(|error| PluginError::new(ErrorCode::ManifestInvalid, parse_reason(&error), error.to_string()))
}

fn parse_reason(error: &serde_json::Error) -> &'static str {
    use serde_json::error::Category;

    match error.classify() {
        Category::Syntax | Category::Eof => "syntax",
        Category::Io => "io",
        Category::Data => {
            let message = error.to_string();
            if message.contains("unknown field") {
                "unknown_field"
            }
            else if message.contains("missing field") {
                "missing_field"
            }
            else if message.contains("duplicate field") {
                "duplicate_field"
            }
            else if message.contains("invalid type") {
                "invalid_type"
            }
            else if message.contains("invalid value") {
                "invalid_value"
            }
            else {
                "invalid_shape"
            }
        }
    }
}

enum Frame {
    Object(HashSet<String>),
    Array,
}

/// 拒绝重复键。`serde_json` 默认后者覆盖前者，这里按原文扫描对象层级。
fn scan_duplicate_keys(text: &str) -> Result<(), PluginError> {
    let bytes = text.as_bytes();
    let mut frames: Vec<Frame> = Vec::new();
    let mut expect_key = false;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index + 1;
                let mut cursor = start;
                while cursor < bytes.len() {
                    match bytes[cursor] {
                        b'\\' => cursor += 2,
                        b'"' => break,
                        _ => cursor += 1,
                    }
                }
                if cursor >= bytes.len() {
                    return Ok(());
                }
                if expect_key && let Some(Frame::Object(keys)) = frames.last_mut() {
                    let key = &text[start..cursor];
                    if !keys.insert(key.to_owned()) {
                        return Err(manifest_invalid("duplicate_key", format!("`{MANIFEST_FILE}` declares duplicate key `{key}`")));
                    }
                }
                expect_key = false;
                index = cursor + 1;
            }
            b'{' => {
                frames.push(Frame::Object(HashSet::new()));
                expect_key = true;
                index += 1;
            }
            b'[' => {
                frames.push(Frame::Array);
                expect_key = false;
                index += 1;
            }
            b'}' | b']' => {
                frames.pop();
                expect_key = false;
                index += 1;
            }
            b',' => {
                expect_key = matches!(frames.last(), Some(Frame::Object(_)));
                index += 1;
            }
            _ => index += 1,
        }
    }
    Ok(())
}

/// 校验清单的全部字段；任何一项失败都拒绝整包。
pub fn validate_manifest(manifest: &PluginManifest, context: &PackContext<'_>) -> Result<(), PluginError> {
    match manifest.schema.as_deref() {
        Some(PLUGIN_SCHEMA) => {}
        Some(other) => {
            return Err(PluginError::new(
                ErrorCode::VersionUnsupported,
                "schema_unsupported",
                format!("unsupported plugin schema `{other}`, expected `{PLUGIN_SCHEMA}`"),
            ));
        }
        None => {
            return Err(PluginError::new(ErrorCode::VersionUnsupported, "schema_missing", format!("`schema` must be `{PLUGIN_SCHEMA}`")));
        }
    }

    if PluginKind::parse(&manifest.kind)? != PluginKind::Rules {
        return Err(kind_unsupported(&manifest.kind));
    }

    validate_pack_id(&manifest.id)?;
    if manifest.name.trim().is_empty() {
        return Err(manifest_invalid("empty_name", "`name` must not be empty"));
    }
    if !is_semver(&manifest.version) {
        return Err(manifest_invalid("invalid_version", format!("`version` of `{}` is not semver", manifest.id)));
    }
    validate_source(&manifest.source)?;
    validate_applies_to(&manifest.applies_to, context.apps)?;
    validate_manifest_scope(&manifest.scope)?;

    if manifest.rules.is_empty() {
        return Err(manifest_invalid("rules_empty", format!("plugin `{}` declares no rules", manifest.id)));
    }

    let mut seen: HashSet<&str> = HashSet::new();
    for rule in &manifest.rules {
        validate_rule(rule, context)?;
        if !seen.insert(rule.id.as_str()) {
            return Err(PluginError::new(
                ErrorCode::RuleConflict,
                "duplicate_rule_id",
                format!("rule id `{}` is declared more than once in this pack", rule.id),
            ));
        }
    }
    Ok(())
}

fn validate_pack_id(id: &str) -> Result<(), PluginError> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')) {
        return Err(manifest_invalid("invalid_plugin_id", "`id` must be a reverse-domain identifier"));
    }
    if id == BUILTIN_NAMESPACE || id.starts_with("builtin.") {
        return Err(manifest_invalid("reserved_plugin_id", format!("`{BUILTIN_NAMESPACE}` is reserved for builtin sources")));
    }
    if !id.contains('.') || id.starts_with('.') || id.ends_with('.') {
        return Err(manifest_invalid("invalid_plugin_id", "`id` must be a reverse-domain identifier"));
    }
    Ok(())
}

fn validate_source(source: &PluginSource) -> Result<(), PluginError> {
    if source.uri.trim().is_empty() {
        return Err(manifest_invalid("source_uri_empty", "`source.uri` must not be empty"));
    }
    if !is_sha256(&source.sha256) {
        return Err(manifest_invalid("source_sha256_invalid", "`source.sha256` must be 64 lowercase hex characters"));
    }
    if matches!(source.origin, PluginOrigin::Local) {
        let path = Path::new(&source.uri);
        if path.is_absolute() || has_drive_prefix(&source.uri) || source.uri.starts_with("\\\\") {
            return Err(manifest_invalid("source_uri_absolute", "`source.uri` must be relative to the project root"));
        }
        if path.components().any(|component| matches!(component, Component::ParentDir)) {
            return Err(manifest_invalid("source_uri_escape", "`source.uri` must not contain `..`"));
        }
    }
    Ok(())
}

fn validate_applies_to(applies_to: &[String], apps: &[&str]) -> Result<(), PluginError> {
    if applies_to.is_empty() {
        return Err(manifest_invalid("applies_to_empty", "`appliesTo` must list at least one application"));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for app in applies_to {
        if app.trim().is_empty() {
            return Err(manifest_invalid("applies_to_empty", "`appliesTo` must not contain empty ids"));
        }
        if !seen.insert(app.as_str()) {
            return Err(manifest_invalid("applies_to_duplicate", format!("`appliesTo` repeats `{app}`")));
        }
        if !apps.contains(&app.as_str()) {
            return Err(manifest_invalid("app_unknown", format!("`appliesTo` declares unknown application `{app}`")));
        }
    }
    Ok(())
}

fn validate_rule(rule: &Rule, context: &PackContext<'_>) -> Result<(), PluginError> {
    if rule.id.trim().is_empty() || !rule.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')) {
        return Err(manifest_invalid("invalid_rule_id", "rule `id` must be a stable identifier"));
    }
    if rule.id == BUILTIN_NAMESPACE || rule.id.starts_with("builtin.") {
        return Err(PluginError::new(
            ErrorCode::RuleConflict,
            "builtin_namespace",
            format!("rule id `{}` uses the reserved `{BUILTIN_NAMESPACE}` namespace", rule.id),
        ));
    }
    if context.builtin_rule_ids.contains(&rule.id.as_str()) {
        return Err(PluginError::new(
            ErrorCode::RuleConflict,
            "builtin_rule_duplicate",
            format!("rule id `{}` conflicts with a builtin rule", rule.id),
        ));
    }
    if rule.version.trim().is_empty() {
        return Err(manifest_invalid("empty_rule_version", format!("rule `{}` must declare a version", rule.id)));
    }
    if rule.domain.trim().is_empty() {
        return Err(manifest_invalid("empty_domain", format!("rule `{}` must declare a domain", rule.id)));
    }
    if !context.domains.contains(&rule.domain.as_str()) {
        return Err(PluginError::new(
            ErrorCode::DomainUnsupported,
            "domain_unknown",
            format!("rule `{}` declares unknown domain `{}`", rule.id, rule.domain),
        ));
    }

    if rule.matching.all.is_empty() {
        return Err(manifest_invalid("empty_match", format!("rule `{}` must declare at least one matcher condition", rule.id)));
    }
    for condition in &rule.matching.all {
        condition.validate(&rule.id)?;
    }

    for field in &rule.evidence {
        if !EVIDENCE_FIELDS.contains(&field.as_str()) {
            return Err(manifest_invalid(
                "unsupported_evidence",
                format!("rule `{}` declares evidence field `{field}` outside the whitelist", rule.id),
            ));
        }
    }

    if rule.disposition.request.trim().is_empty() {
        return Err(manifest_invalid("empty_disposition_request", format!("rule `{}` must declare `disposition.request`", rule.id)));
    }
    if rule.disposition.requires.iter().any(|value| value.trim().is_empty()) {
        return Err(manifest_invalid(
            "empty_disposition_requires",
            format!("rule `{}` declares an empty `disposition.requires` entry", rule.id),
        ));
    }
    if rule.disposition.on_failure.trim().is_empty() {
        return Err(manifest_invalid("empty_disposition_on_failure", format!("rule `{}` must declare `disposition.onFailure`", rule.id)));
    }
    Ok(())
}

fn is_semver(value: &str) -> bool {
    let core = value.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) && (part.len() == 1 || !part.starts_with('0')))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn has_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn manifest_invalid(reason: &str, message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorCode::ManifestInvalid, reason, message)
}

fn io_failed(reason: &str, message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorCode::IoFailed, reason, message)
}

#[cfg(test)]
mod tests {
    use std::{
        process,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use serde_json::json;

    use super::*;
    use crate::scope::DEFAULT_SCOPE_MAX_DEPTH;

    const SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    struct TempPack {
        dir: PathBuf,
    }

    impl TempPack {
        fn new(contents: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!("wae-plugins-test-{}-{}", process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(MANIFEST_FILE), contents).unwrap();
            Self { dir }
        }

        fn with(entity: &serde_json::Value) -> Self {
            Self::new(&serde_json::to_string(entity).unwrap())
        }
    }

    impl Drop for TempPack {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn manifest() -> serde_json::Value {
        json!({
            "schema": PLUGIN_SCHEMA,
            "kind": "rules",
            "id": "com.example.cleaner-rules",
            "name": "Example Cleaner Rules",
            "version": "1.0.0",
            "source": { "origin": "local", "uri": "./plugins/cleaner-rules", "sha256": SHA256 },
            "appliesTo": ["open-cleaner"],
            "scope": {
                "roots": ["."],
                "excludes": [".git/**"],
                "maxDepth": 6,
                "maxBytes": 1048576
            },
            "rules": [{
                "id": "dev-artifact-node-modules",
                "version": "1",
                "domain": "dev-artifacts",
                "kind": "cleanup",
                "match": { "all": [{ "type": "directory" }, { "nameIn": ["node_modules"] }] },
                "evidence": ["anchor", "path", "sizeBytes"],
                "disposition": { "request": "trash", "requires": ["anchor-present"], "onFailure": "skip" },
                "defaultSelected": false
            }]
        })
    }

    fn context() -> PackContext<'static> {
        PackContext::new(&["open-cleaner", "open-finder"], &["dev-artifacts"], &[])
    }

    fn assert_error(pack: &TempPack, code: ErrorCode, reason: &str) {
        let error = load_pack(&pack.dir, &context()).unwrap_err();
        assert_eq!(error.code, code, "unexpected code for reason {}", error.reason);
        assert_eq!(error.reason, reason);
    }

    #[test]
    fn loads_a_valid_pack() {
        let pack = TempPack::with(&manifest());
        let loaded = load_pack(&pack.dir, &context()).unwrap();
        assert_eq!(loaded.manifest.id, "com.example.cleaner-rules");
        assert_eq!(loaded.manifest.rules.len(), 1);
        assert_eq!(loaded.manifest.scope.max_depth, DEFAULT_SCOPE_MAX_DEPTH);
        assert!(loaded.dir.is_absolute());
        assert!(!loaded.manifest_bytes.is_empty());
    }

    #[test]
    fn rejects_unknown_fields() {
        let mut entity = manifest();
        entity["priority"] = json!(1);
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::ManifestInvalid, "unknown_field");
    }

    #[test]
    fn rejects_duplicate_keys() {
        let text = format!(r#"{{"schema":"{PLUGIN_SCHEMA}","schema":"{PLUGIN_SCHEMA}"}}"#);
        let pack = TempPack::new(&text);
        assert_error(&pack, ErrorCode::ManifestInvalid, "duplicate_key");
    }

    #[test]
    fn rejects_missing_schema_as_unsupported_version() {
        let mut entity = manifest();
        entity.as_object_mut().unwrap().remove("schema");
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::VersionUnsupported, "schema_missing");
    }

    #[test]
    fn rejects_future_schema() {
        let mut entity = manifest();
        entity["schema"] = json!("wae.plugin/v1");
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::VersionUnsupported, "schema_unsupported");
    }

    #[test]
    fn rejects_code_kind() {
        let mut entity = manifest();
        entity["kind"] = json!("code");
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::KindUnsupported, "kind_unsupported");
    }

    #[test]
    fn rejects_builtin_namespace() {
        let mut entity = manifest();
        entity["rules"][0]["id"] = json!("builtin.node-semver");
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::RuleConflict, "builtin_namespace");
    }

    #[test]
    fn rejects_duplicate_rule_ids() {
        let mut entity = manifest();
        let rule = entity["rules"][0].clone();
        entity["rules"].as_array_mut().unwrap().push(rule);
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::RuleConflict, "duplicate_rule_id");
    }

    #[test]
    fn rejects_unknown_domain() {
        let mut entity = manifest();
        entity["rules"][0]["domain"] = json!("archive-temp");
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::DomainUnsupported, "domain_unknown");
    }

    #[test]
    fn rejects_evidence_outside_the_whitelist() {
        let mut entity = manifest();
        entity["rules"][0]["evidence"] = json!(["contentHash"]);
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::ManifestInvalid, "unsupported_evidence");
    }

    #[test]
    fn rejects_unknown_application() {
        let mut entity = manifest();
        entity["appliesTo"] = json!(["open-zip"]);
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::ManifestInvalid, "app_unknown");
    }

    #[test]
    fn rejects_two_matchers_in_one_condition() {
        let mut entity = manifest();
        entity["rules"][0]["match"]["all"] = json!([{ "type": "directory", "nameEquals": "node_modules" }]);
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::ManifestInvalid, "match_condition_conflict");
    }

    #[test]
    fn rejects_scope_escape() {
        let mut entity = manifest();
        entity["scope"]["roots"] = json!(["../outside"]);
        let pack = TempPack::with(&entity);
        assert_error(&pack, ErrorCode::ScopeInvalid, "parent_escape");
    }

    #[test]
    fn rejects_missing_manifest() {
        let pack = TempPack::new("{}");
        fs::remove_file(pack.dir.join(MANIFEST_FILE)).unwrap();
        assert_error(&pack, ErrorCode::IoFailed, "manifest_missing");
    }

    #[test]
    fn rejects_non_utf8_manifest() {
        let pack = TempPack::new("{}");
        fs::write(pack.dir.join(MANIFEST_FILE), [0xff, 0xfe]).unwrap();
        assert_error(&pack, ErrorCode::ManifestInvalid, "non_utf8");
    }

    #[test]
    fn rejects_comments_and_trailing_commas() {
        let pack = TempPack::new("{\"schema\": \"wae.plugin/v0\",}");
        assert_error(&pack, ErrorCode::ManifestInvalid, "syntax");
    }
}
