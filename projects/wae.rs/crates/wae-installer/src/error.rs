//! 安装包封装的失败分类；每个变体都有稳定 `code()`，下游按字段判别，
//! 禁止匹配错误字符串。平台或格式缺少能力时用 [`InstallerError::Unsupported`]，
//! 不伪造成功（与 `wae-harness` / `wae-webview` 同一口径）。

/// 编译目标平台；与 `wae::webview::Platform` 同形，独立定义以保持本 crate
/// 不依赖 WebView 合同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    /// Windows。
    Windows,
    /// macOS。
    Macos,
    /// Linux。
    Linux,
    /// 其它目标（无安装包能力）。
    Other,
}

/// 当前编译目标平台。
pub fn current_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    }
    else if cfg!(target_os = "macos") {
        Platform::Macos
    }
    else if cfg!(target_os = "linux") {
        Platform::Linux
    }
    else {
        Platform::Other
    }
}

/// 稳定错误码常量；`code()` 与 napi 映射都引用这里。
pub mod codes {
    /// I/O 失败。
    pub const IO: &str = "installer_io";
    /// 规格非法（版本、payload、GUID 等），变体自带更细的码。
    pub const SPEC: &str = "installer_spec";
    /// 格式或平台缺少能力。
    pub const UNSUPPORTED: &str = "installer_unsupported";
    /// 内部构造失败（布局、命名冲突、路径超长等）。
    pub const LAYOUT: &str = "installer_layout";
    /// 产物回读不一致。
    pub const VERIFY: &str = "installer_verify";
}

/// 安装包封装失败分类。
#[derive(Debug)]
pub enum InstallerError {
    /// 读写产物或 payload 时的 I/O 失败。
    Io(std::io::Error),
    /// 规格非法；`code` 是稳定子码（如 `installer_spec_version`）。
    InvalidSpec {
        /// 稳定子码。
        code: &'static str,
        /// 说明。
        message: String,
    },
    /// 格式或平台缺少能力。
    Unsupported {
        /// 缺少的能力（通常是格式名，如 `msix` / `pkg` / `dmg`）。
        capability: &'static str,
        /// 编译目标平台。
        platform: Platform,
    },
    /// 内部构造失败：布局、8.3 命名冲突、tar 路径超长等。
    Layout(String),
    /// 产物回读与期望不一致。
    Verify(String),
}

impl InstallerError {
    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => codes::IO,
            Self::InvalidSpec { code, .. } => code,
            Self::Unsupported { .. } => codes::UNSUPPORTED,
            Self::Layout(_) => codes::LAYOUT,
            Self::Verify(_) => codes::VERIFY,
        }
    }

    /// 构造规格非法错误（稳定子码 + 说明）。
    pub fn spec(code: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidSpec { code, message: message.into() }
    }

    /// 构造结构化的能力缺失错误。
    pub fn unsupported(capability: &'static str) -> Self {
        Self::Unsupported { capability, platform: current_platform() }
    }
}

impl std::fmt::Display for InstallerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "io: {error}"),
            Self::InvalidSpec { code, message } => write!(formatter, "{code}: {message}"),
            Self::Unsupported { capability, platform } => {
                write!(formatter, "capability `{capability}` is not supported on platform {platform:?}")
            }
            Self::Layout(message) => write!(formatter, "layout: {message}"),
            Self::Verify(message) => write!(formatter, "verify: {message}"),
        }
    }
}

impl std::error::Error for InstallerError {}

impl From<std::io::Error> for InstallerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// 安装包封装结果别名。
pub type Result<T> = std::result::Result<T, InstallerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let io = InstallerError::Io(std::io::Error::other("disk"));
        assert_eq!(io.code(), "installer_io");
        let spec = InstallerError::spec("installer_spec_version", "bad version");
        assert_eq!(spec.code(), "installer_spec_version");
        let unsupported = InstallerError::unsupported("pkg");
        assert_eq!(unsupported.code(), "installer_unsupported");
        assert_eq!(InstallerError::Layout("x".into()).code(), "installer_layout");
        assert_eq!(InstallerError::Verify("x".into()).code(), "installer_verify");
    }

    #[test]
    fn unsupported_carries_current_platform() {
        let error = InstallerError::unsupported("msix");
        match error {
            InstallerError::Unsupported { capability, platform } => {
                assert_eq!(capability, "msix");
                assert_eq!(platform, current_platform());
            }
            other => panic!("unexpected variant: {other}"),
        }
    }
}
