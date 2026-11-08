//! 作用域解析、运行期交集收窄与候选越界判定。
//!
//! 插件声明只能收窄扫描请求，不能放宽。候选路径必须同时落在请求根与插件根内，
//! 且满足排除项、深度、体积与符号链接约束；违反只丢弃该候选并记
//! `plugin_scope_violation`，不自动裁剪、不扩大作用域。

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, PluginError};

/// `scope.maxDepth` 默认值。
pub const DEFAULT_SCOPE_MAX_DEPTH: u8 = 6;

/// `scope.maxBytes` 默认值（1 MiB）。
pub const DEFAULT_SCOPE_MAX_BYTES: u64 = 1_048_576;

/// `scope.maxBytes` 的上界（2^53）。
const MAX_SCOPE_BYTES: u64 = 1 << 53;

/// 插件清单作用域。字段是声明，不是请求；运行期只能收窄。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestScope {
    /// 相对项目根的根目录，最多 8 项。
    pub roots: Vec<String>,
    /// 相对根的排除 glob，最多 64 项。
    #[serde(default)]
    pub excludes: Vec<String>,
    /// 最大深度。
    #[serde(default = "default_scope_max_depth")]
    pub max_depth: u8,
    /// 单个候选的最大体积。
    #[serde(default = "default_scope_max_bytes")]
    pub max_bytes: u64,
}

fn default_scope_max_depth() -> u8 {
    DEFAULT_SCOPE_MAX_DEPTH
}

fn default_scope_max_bytes() -> u64 {
    DEFAULT_SCOPE_MAX_BYTES
}

/// 扫描请求的作用域。运行期只看这一个来源。
#[derive(Debug, Clone)]
pub struct ScanScope {
    /// 请求根，必须非空。
    pub roots: Vec<PathBuf>,
    /// 请求排除项，相对请求根。
    pub excludes: Vec<String>,
    /// 请求最大深度。
    pub max_depth: u8,
    /// 请求最大体积。
    pub max_bytes: u64,
}

impl ScanScope {
    /// 用单个根构造默认请求作用域。
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { roots: vec![root.into()], excludes: Vec::new(), max_depth: 32, max_bytes: u64::MAX }
    }
}

/// 交集解析结果。
#[derive(Debug, Clone)]
pub enum ScopeResolution {
    /// 插件与请求有交集；后续按收窄后的作用域判定候选。
    Active(ResolvedScope),
    /// 无交集或请求未启用该根；该包本次跳过并记录 `plugin_scope_excluded`。
    Excluded(&'static str),
}

/// 收窄后的作用域。
#[derive(Debug, Clone)]
pub struct ResolvedScope {
    /// 生效根与对应的排除项。
    pub roots: Vec<ResolvedRoot>,
    /// 收窄后的最大深度。
    pub max_depth: u8,
    /// 收窄后的最大体积。
    pub max_bytes: u64,
}

/// 一个生效根。
#[derive(Debug, Clone)]
pub struct ResolvedRoot {
    /// 规范化后的绝对根。
    pub path: PathBuf,
    /// 相对该根的排除 glob。
    pub excludes: Vec<String>,
}

/// 候选进入作用域判定的最小事实。
#[derive(Debug, Clone, Copy)]
pub struct PathFacts {
    /// 文件或目录统计体积。
    pub size_bytes: u64,
    /// 是否符号链接/junction。
    pub symlink: bool,
}

/// 校验清单里的作用域字段。
pub fn validate_manifest_scope(scope: &ManifestScope) -> Result<(), PluginError> {
    if scope.roots.is_empty() {
        return Err(invalid("roots_empty", "`scope.roots` must not be empty"));
    }
    if scope.roots.len() > 8 {
        return Err(invalid("too_many_roots", "`scope.roots` allows at most 8 entries"));
    }
    for root in &scope.roots {
        check_relative_pattern(root, "root")?;
        if root.contains('*') || root.contains('?') || root.contains('[') || root.contains(']') {
            return Err(invalid("glob_root", "`scope.roots` must not contain glob characters"));
        }
    }
    if scope.excludes.len() > 64 {
        return Err(invalid("too_many_excludes", "`scope.excludes` allows at most 64 entries"));
    }
    for exclude in &scope.excludes {
        check_relative_pattern(exclude, "exclude")?;
    }
    if !(1..=32).contains(&scope.max_depth) {
        return Err(invalid("depth_out_of_range", "`scope.maxDepth` must be between 1 and 32"));
    }
    if scope.max_bytes == 0 || scope.max_bytes > MAX_SCOPE_BYTES {
        return Err(invalid("bytes_out_of_range", "`scope.maxBytes` must be between 1 and 2^53"));
    }
    Ok(())
}

fn check_relative_pattern(value: &str, field: &str) -> Result<(), PluginError> {
    let path = Path::new(value);
    if value.is_empty() {
        return Err(invalid("empty_path", format!("`scope.{field}` must not be empty")));
    }
    if path.is_absolute() || value.starts_with("\\\\") || has_drive_prefix(value) {
        return Err(invalid("absolute_path", format!("`scope.{field}` must be relative")));
    }
    if path.components().any(|component| matches!(component, Component::ParentDir)) {
        return Err(invalid("parent_escape", format!("`scope.{field}` must not contain `..`")));
    }
    Ok(())
}

fn has_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn invalid(reason: &str, message: impl Into<String>) -> PluginError {
    PluginError::new(ErrorCode::ScopeInvalid, reason, message)
}

/// 计算请求与插件作用域的交集。
///
/// `project_root` 是插件相对根的基准；请求根是绝对路径。请求只能收窄：交集取
/// 两者中更深的一侧，绝不把插件根扩到请求根之外。
pub fn resolve(scope: &ManifestScope, request: &ScanScope, project_root: &Path) -> Result<ScopeResolution, PluginError> {
    if request.roots.is_empty() {
        return Err(invalid("request_roots_empty", "scan request must declare at least one root"));
    }

    let mut roots = Vec::new();
    let request_roots: Vec<PathBuf> = request.roots.iter().map(|root| normalize_lexically(root)).collect();
    for declared in &scope.roots {
        let declared_abs = normalize_lexically(&project_root.join(declared));
        for request_root in &request_roots {
            if let Some((effective, prefix)) = narrow(&declared_abs, request_root) {
                let mut excludes = scope.excludes.clone();
                if !prefix.is_empty() {
                    excludes = excludes.into_iter().map(|pattern| format!("{prefix}/{pattern}")).collect();
                }
                excludes.extend(request.excludes.iter().cloned());
                roots.push(ResolvedRoot { path: effective, excludes });
            }
        }
    }

    if roots.is_empty() {
        return Ok(ScopeResolution::Excluded("plugin_scope_excluded"));
    }

    let depth_penalty = scope.max_depth as u32;
    let max_depth = depth_penalty.min(request.max_depth as u32) as u8;
    let max_bytes = scope.max_bytes.min(request.max_bytes);
    Ok(ScopeResolution::Active(ResolvedScope { roots, max_depth, max_bytes }))
}

/// 去掉 `.` 组件，保留前缀、根与普通组件。用于比较前统一拼写。
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 返回两者中更深的根，以及相对更深根的路径（用于排除项前缀）。
fn narrow<'a>(a: &'a Path, b: &'a Path) -> Option<(PathBuf, String)> {
    if b.starts_with(a) {
        let prefix = b.strip_prefix(a).ok()?;
        Some((b.to_path_buf(), slash_path(prefix)))
    }
    else if a.starts_with(b) {
        Some((a.to_path_buf(), String::new()))
    }
    else {
        None
    }
}

fn slash_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl ResolvedScope {
    /// 判断候选是否在作用域内；越界返回 `plugin_scope_violation`。
    pub fn accepts(&self, path: &Path, facts: PathFacts) -> Result<(), PluginError> {
        let Some(root) = self.roots.iter().find(|root| path.starts_with(&root.path))
        else {
            return Err(PluginError::new(ErrorCode::ScopeViolation, "out_of_root", "candidate is outside the declared roots"));
        };

        let relative = path.strip_prefix(&root.path).unwrap_or(path);
        let depth = relative.components().filter(|component| matches!(component, Component::Normal(_))).count();
        if depth > self.max_depth as usize {
            return Err(PluginError::new(ErrorCode::ScopeViolation, "exceeds_depth", "candidate exceeds `maxDepth`"));
        }
        if facts.size_bytes > self.max_bytes {
            return Err(PluginError::new(ErrorCode::ScopeViolation, "exceeds_bytes", "candidate exceeds `maxBytes`"));
        }
        if facts.symlink {
            return Err(PluginError::new(ErrorCode::ScopeViolation, "symlink", "candidate is a symlink and is not followed"));
        }

        let relative = slash_path(relative);
        if root.excludes.iter().any(|pattern| glob_match(pattern, &relative)) {
            return Err(PluginError::new(ErrorCode::ScopeViolation, "excluded", "candidate matches an exclude pattern"));
        }
        Ok(())
    }
}

/// 闭集 glob：只支持 `*`（不跨分隔符）、`**`（跨分隔符）与字面字符。
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    let text = text.replace('\\', "/");
    glob_bytes(pattern.as_bytes(), text.as_bytes())
}

fn glob_bytes(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' if pattern.get(1) == Some(&b'*') => {
            let rest = pattern.iter().position(|byte| *byte != b'*').map_or(&[][..], |index| &pattern[index..]);
            if glob_bytes(rest, text) {
                return true;
            }
            text.iter().enumerate().any(|(index, _)| glob_bytes(rest, &text[index + 1..]))
        }
        b'*' => glob_bytes(&pattern[1..], text) || (!text.is_empty() && text[0] != b'/' && glob_bytes(pattern, &text[1..])),
        byte if text.first() == Some(&byte) => glob_bytes(&pattern[1..], &text[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(roots: Vec<&str>, excludes: Vec<&str>, depth: u8, bytes: u64) -> ManifestScope {
        ManifestScope {
            roots: roots.into_iter().map(str::to_owned).collect(),
            excludes: excludes.into_iter().map(str::to_owned).collect(),
            max_depth: depth,
            max_bytes: bytes,
        }
    }

    #[test]
    fn rejects_absolute_and_parent_roots() {
        assert_eq!(validate_manifest_scope(&scope(vec!["C:/work"], vec![], 6, 100)).unwrap_err().reason, "absolute_path");
        assert_eq!(validate_manifest_scope(&scope(vec!["../outside"], vec![], 6, 100)).unwrap_err().reason, "parent_escape");
        assert_eq!(validate_manifest_scope(&scope(vec!["src/**"], vec![], 6, 100)).unwrap_err().reason, "glob_root");
        assert_eq!(validate_manifest_scope(&scope(vec![], vec![], 6, 100)).unwrap_err().reason, "roots_empty");
    }

    #[test]
    fn request_narrows_the_plugin_root() {
        let plugin = scope(vec!["src"], vec![], 6, 100);
        let project = PathBuf::from("C:/work");
        let request = ScanScope::from_root("C:/work/src/app");
        let ScopeResolution::Active(resolved) = resolve(&plugin, &request, &project).unwrap()
        else {
            panic!("expected active scope");
        };
        assert_eq!(resolved.roots[0].path, PathBuf::from("C:/work/src/app"));
        assert_eq!(resolved.max_depth, 6);
    }

    #[test]
    fn plugin_scope_never_escapes_request_root() {
        let plugin = scope(vec!["."], vec![], 6, 100);
        let project = PathBuf::from("C:/work");
        let request = ScanScope::from_root("C:/work/src");
        let ScopeResolution::Active(resolved) = resolve(&plugin, &request, &project).unwrap()
        else {
            panic!("expected active scope");
        };
        let error = resolved.accepts(Path::new("C:/work/other/file"), PathFacts { size_bytes: 4, symlink: false }).unwrap_err();
        assert_eq!(error.code, ErrorCode::ScopeViolation);
        assert_eq!(error.reason, "out_of_root");
    }

    #[test]
    fn candidate_limits_and_excludes_are_per_candidate() {
        let plugin = scope(vec!["."], vec!["vendor/**", "*.tmp"], 2, 8);
        let project = PathBuf::from("C:/work");
        let request = ScanScope::from_root("C:/work");
        let ScopeResolution::Active(resolved) = resolve(&plugin, &request, &project).unwrap()
        else {
            panic!("expected active scope");
        };
        let facts = PathFacts { size_bytes: 8, symlink: false };
        assert!(resolved.accepts(Path::new("C:/work/a/b"), facts).is_ok());
        assert_eq!(resolved.accepts(Path::new("C:/work/a/b/c"), facts).unwrap_err().reason, "exceeds_depth");
        assert_eq!(resolved.accepts(Path::new("C:/work/vendor/a"), facts).unwrap_err().reason, "excluded");
        assert_eq!(resolved.accepts(Path::new("C:/work/a.tmp"), facts).unwrap_err().reason, "excluded");
        assert_eq!(resolved.accepts(Path::new("C:/work/a"), PathFacts { size_bytes: 9, symlink: false }).unwrap_err().reason, "exceeds_bytes");
        assert_eq!(resolved.accepts(Path::new("C:/work/a"), PathFacts { size_bytes: 1, symlink: true }).unwrap_err().reason, "symlink");
    }

    #[test]
    fn glob_matches_documented_subset() {
        assert!(glob_match("vendor/**", "vendor/a/b"));
        assert!(glob_match("*.tmp", "a.tmp"));
        assert!(!glob_match("*.tmp", "dir/a.tmp"));
        assert!(glob_match("dir/*", "dir/a"));
        assert!(!glob_match("dir/*", "dir/a/b"));
    }
}
