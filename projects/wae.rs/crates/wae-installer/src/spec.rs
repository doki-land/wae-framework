//! 安装包规格：产品元数据、payload、快捷方式与各格式选项。
//!
//! 这是 napi / TS 共用的 wire 形状：结构体字段 `camelCase`，枚举 tag
//! `kebab-case`（工作区 Rust wire 命名规则）。规格在进入任何 writer 之前
//! 必须先通过 [`InstallerSpec::validate`]，失败返回稳定子码。

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{InstallerError, Result};

/// 规格校验子码。
pub mod spec_codes {
    /// `ProductVersion` 不是严格 `X.Y.Z`。
    pub const VERSION: &str = "installer_spec_version";
    /// payload 为空。
    pub const PAYLOAD_EMPTY: &str = "installer_spec_payload_empty";
    /// payload 文件名含非 ASCII（MSI 字符串池按 ANSI 代码页）。
    pub const PAYLOAD_NAME: &str = "installer_spec_payload_name";
    /// GUID 字段形状非法。
    pub const GUID: &str = "installer_spec_guid";
    /// 必填文本为空。
    pub const EMPTY_FIELD: &str = "installer_spec_empty_field";
}

/// 产品元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductMeta {
    /// 展示名，如 `MyJarvis`。
    pub name: String,
    /// 严格 `X.Y.Z`（不含 `v` 前缀）。
    pub version: String,
    /// 厂商名（MSI Manufacturer）。
    pub manufacturer: String,
    /// MSI `ProductLanguage` LCID；默认 1033（en-US）。
    #[serde(default = "default_language")]
    pub language: u16,
    /// 升级标识 GUID；一旦发布永不变更。缺省时由确定性派生，作者必须回填固定。
    #[serde(default)]
    pub upgrade_code: Option<String>,
    /// 产品 GUID；缺省由 `upgradeCode` 命名空间 + `name` + `version` 派生。
    #[serde(default)]
    pub product_code: Option<String>,
    /// 包 GUID；缺省由 `upgradeCode` 命名空间 + 内容指纹派生。
    #[serde(default)]
    pub package_code: Option<String>,
    /// `.ico` 路径（ARP 图标与 Shortcut 图标）。
    #[serde(default)]
    pub icon: Option<PathBuf>,
    /// Add/Remove Programs 补充字段。
    #[serde(default)]
    pub arp: ArpMeta,
}

impl ProductMeta {
    /// 校验版本、GUID 形状与必填文本。
    pub fn validate(&self) -> Result<()> {
        if !is_exact_version(&self.version) {
            return Err(InstallerError::spec(spec_codes::VERSION, format!("`version` must be X.Y.Z, got `{}`", self.version)));
        }
        if self.name.trim().is_empty() {
            return Err(InstallerError::spec(spec_codes::EMPTY_FIELD, "`product.name` is required"));
        }
        if self.manufacturer.trim().is_empty() {
            return Err(InstallerError::spec(spec_codes::EMPTY_FIELD, "`product.manufacturer` is required"));
        }
        for (field, value) in [
            ("upgradeCode", self.upgrade_code.as_deref()),
            ("productCode", self.product_code.as_deref()),
            ("packageCode", self.package_code.as_deref()),
        ] {
            if let Some(guid) = value
                && !is_guid_text(guid)
            {
                return Err(InstallerError::spec(spec_codes::GUID, format!("`product.{field}` is not a GUID: `{guid}`")));
            }
        }
        Ok(())
    }
}

/// `X.Y.Z` 严格三段数字版本。
pub fn is_exact_version(text: &str) -> bool {
    let mut parts = text.split('.');
    let (Some(major), Some(minor), Some(patch), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    [major, minor, patch].iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// GUID 文本形状：8-4-4-4-12 十六进制，可带花括号。
pub fn is_guid_text(text: &str) -> bool {
    let trimmed = text.trim().trim_matches('{').trim_matches('}');
    let groups: Vec<&str> = trimmed.split('-').collect();
    let lens = [8usize, 4, 4, 4, 12];
    groups.len() == 5 && groups.iter().zip(lens).all(|(group, len)| group.len() == len && group.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Add/Remove Programs 补充字段（全部可选）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArpMeta {
    /// `ARPHELP`：支持链接。
    #[serde(default)]
    pub help_link: Option<String>,
    /// `ARPCOMMENT`：备注。
    #[serde(default)]
    pub comment: Option<String>,
    /// `ARPCONTACT`：联系人。
    #[serde(default)]
    pub contact: Option<String>,
    /// `ARPURLINFOABOUT`：主页。
    #[serde(default)]
    pub url_info_about: Option<String>,
    /// `ARPURLUPDATEINFO`：更新检查。
    #[serde(default)]
    pub url_update_info: Option<String>,
}

/// payload 文件的安装目标目录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirKey {
    /// Program Files（MSI `ProgramFilesFolder`；deb `/usr/bin` 与 `/usr/share/<package>` 由格式映射）。
    ProgramFiles,
    /// 用户 Roaming AppData。
    AppData,
    /// 用户 Local AppData。
    LocalAppData,
    /// 桌面目录。
    DesktopFolder,
    /// 开始菜单程序组。
    StartMenuFolder,
    /// 自定义目录键（相对安装根的路径文本）。
    Custom(String),
}

/// 单个 payload 文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayloadFile {
    /// 主机上的真实文件路径。
    pub source: PathBuf,
    /// 安装目标目录。
    pub target: DirKey,
    /// 安装后的文件名；`None` → `source` 的文件名。要求 ASCII。
    #[serde(default)]
    pub file_name: Option<String>,
    /// MSI `File.Attributes`；默认 0。
    #[serde(default)]
    pub attributes: u16,
    /// 可执行位（deb/rpm 用 0755；MSI 不依赖此位）。默认 false。
    #[serde(default)]
    pub executable: bool,
}

impl PayloadFile {
    /// 安装后的文件名。
    pub fn install_name(&self) -> Result<&str> {
        let name = match &self.file_name {
            Some(name) => name.as_str(),
            None => self.source.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
                InstallerError::spec(spec_codes::PAYLOAD_NAME, format!("`{}` has no usable file name", self.source.display()))
            })?,
        };
        if !name.bytes().all(|byte| byte.is_ascii()) || name.contains('/') || name.contains('\\') || name.is_empty() {
            return Err(InstallerError::spec(
                spec_codes::PAYLOAD_NAME,
                format!("payload name `{name}` must be ASCII and free of path separators"),
            ));
        }
        Ok(name)
    }
}

/// 快捷方式定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shortcut {
    /// 快捷方式显示名。
    pub name: String,
    /// 归属组件键（默认取主 exe 的组件名）。
    pub component: String,
    /// 目标文件名（相对 `INSTALLDIR`）。
    pub target: String,
    /// 启动参数。
    #[serde(default)]
    pub arguments: Option<String>,
    /// 工作目录键。
    #[serde(default)]
    pub working_dir: Option<String>,
    /// 图标文件名（相对 `INSTALLDIR`）。
    #[serde(default)]
    pub icon: Option<String>,
}

/// 安装包规格：所有格式共用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallerSpec {
    /// 产品元数据。
    pub product: ProductMeta,
    /// payload 文件列表；第一个 `executable` 项（或第一项）视为主 exe。
    pub payload: Vec<PayloadFile>,
    /// 快捷方式。
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
    /// 产物输出路径。
    pub output: PathBuf,
}

impl InstallerSpec {
    /// 进入任何 writer 前的统一校验。
    pub fn validate(&self) -> Result<()> {
        self.product.validate()?;
        if self.payload.is_empty() {
            return Err(InstallerError::spec(spec_codes::PAYLOAD_EMPTY, "`payload` must not be empty"));
        }
        for file in &self.payload {
            file.install_name()?;
        }
        Ok(())
    }
}

/// MSI 安装范围。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// 每台机器（需要提权）。
    #[default]
    PerMachine,
    /// 仅当前用户。
    PerUser,
}

/// MSI 选项。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MsiOptions {
    /// 安装范围。
    #[serde(default)]
    pub scope: Scope,
    /// 生成 `Upgrade` 表实现大版本替换（默认 true）。
    #[serde(default = "default_true")]
    pub major_upgrade: bool,
    /// 附加 `Property` 行。
    #[serde(default)]
    pub extra_properties: BTreeMap<String, String>,
}

/// deb 选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebOptions {
    /// 维护者（`Maintainer`）。
    pub maintainer: String,
    /// 长描述（`Description`）。
    pub description: String,
    /// `Section`；默认 `devel`。
    #[serde(default = "default_section")]
    pub section: String,
    /// `Priority`；默认 `optional`。
    #[serde(default = "default_priority")]
    pub priority: String,
    /// `Depends` 列表。
    #[serde(default)]
    pub depends: Vec<String>,
    /// 目标架构（`amd64` / `arm64`）；`None` → 按编译目标推断。
    #[serde(default)]
    pub architecture: Option<String>,
    /// 生成 `postinst` / `prerm`（默认 true，内容为 no-op 安全体）。
    #[serde(default = "default_true")]
    pub scripts: bool,
}

impl Default for DebOptions {
    fn default() -> Self {
        Self {
            maintainer: String::new(),
            description: String::new(),
            section: default_section(),
            priority: default_priority(),
            depends: Vec::new(),
            architecture: None,
            scripts: true,
        }
    }
}

/// rpm 选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RpmOptions {
    /// `Summary`。
    pub summary: String,
    /// `Description`。
    pub description: String,
    /// `License`（SPDX 风格文本）。
    pub license: String,
    /// `Url`。
    #[serde(default)]
    pub url: Option<String>,
    /// `Packager`。
    #[serde(default)]
    pub packager: Option<String>,
    /// `Group`；默认 `Applications/Utilities`。
    #[serde(default = "default_group")]
    pub group: String,
    /// 目标架构（`x86_64` / `aarch64`）；`None` → 按编译目标推断。
    #[serde(default)]
    pub architecture: Option<String>,
}

impl Default for RpmOptions {
    fn default() -> Self {
        Self {
            summary: String::new(),
            description: String::new(),
            license: String::new(),
            url: None,
            packager: None,
            group: default_group(),
            architecture: None,
        }
    }
}

/// MSIX 选项（`cfg(windows)` 路径使用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MsixOptions {
    /// `Identity/Name`：`[A-Za-z0-9.]+`。
    pub identity_name: String,
    /// `Identity/Publisher`：如 `CN=Contoso`。
    pub publisher: String,
    /// 显示名。
    pub display_name: String,
    /// `Capabilities` 列表。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 应用徽标 `.png` 路径。
    #[serde(default)]
    pub logo: Option<PathBuf>,
}

impl MsixOptions {
    /// 校验 identity 形状。
    pub fn validate(&self) -> Result<()> {
        if self.identity_name.is_empty() || !self.identity_name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        {
            return Err(InstallerError::spec(spec_codes::EMPTY_FIELD, format!("`identityName` is invalid: `{}`", self.identity_name)));
        }
        if self.publisher.trim().is_empty() {
            return Err(InstallerError::spec(spec_codes::EMPTY_FIELD, "`publisher` is required"));
        }
        Ok(())
    }
}

fn default_language() -> u16 {
    1033
}

fn default_true() -> bool {
    true
}

fn default_section() -> String {
    "devel".to_owned()
}

fn default_priority() -> String {
    "optional".to_owned()
}

fn default_group() -> String {
    "Applications/Utilities".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product() -> ProductMeta {
        ProductMeta {
            name: "MyJarvis".into(),
            version: "0.1.0".into(),
            manufacturer: "Contoso".into(),
            language: 1033,
            upgrade_code: None,
            product_code: None,
            package_code: None,
            icon: None,
            arp: ArpMeta::default(),
        }
    }

    #[test]
    fn version_must_be_exact() {
        assert!(is_exact_version("0.1.0"));
        assert!(is_exact_version("10.20.30"));
        assert!(!is_exact_version("v0.1.0"));
        assert!(!is_exact_version("0.1"));
        assert!(!is_exact_version("0.1.0.2"));
        assert!(!is_exact_version("0.1.0-rc1"));
        assert!(!is_exact_version(""));

        let mut meta = product();
        meta.version = "0.1".into();
        assert_eq!(meta.validate().unwrap_err().code(), spec_codes::VERSION);
    }

    #[test]
    fn guid_shape() {
        assert!(is_guid_text("00112233-4455-6677-8899-aabbccddeeff"));
        assert!(is_guid_text("{00112233-4455-6677-8899-AABBCCDDEEFF}"));
        assert!(!is_guid_text("00112233-4455-6677-8899-aabbccddee"));
        assert!(!is_guid_text("not-a-guid"));

        let mut meta = product();
        meta.upgrade_code = Some("zz".into());
        assert_eq!(meta.validate().unwrap_err().code(), spec_codes::GUID);
    }

    #[test]
    fn payload_names_must_be_ascii() {
        let spec = InstallerSpec {
            product: product(),
            payload: vec![PayloadFile {
                source: "build/我的应用.exe".into(),
                target: DirKey::ProgramFiles,
                file_name: None,
                attributes: 0,
                executable: true,
            }],
            shortcuts: Vec::new(),
            output: "out/app.msi".into(),
        };
        assert_eq!(spec.validate().unwrap_err().code(), spec_codes::PAYLOAD_NAME);

        let empty = InstallerSpec { product: product(), payload: Vec::new(), shortcuts: Vec::new(), output: "out/app.msi".into() };
        assert_eq!(empty.validate().unwrap_err().code(), spec_codes::PAYLOAD_EMPTY);
    }

    #[test]
    fn wire_is_camel_case_and_kebab_tags() {
        let json = serde_json::to_value(PayloadFile {
            source: "a/exe".into(),
            target: DirKey::StartMenuFolder,
            file_name: Some("app.exe".into()),
            attributes: 0,
            executable: true,
        })
        .unwrap();
        assert_eq!(json["fileName"], "app.exe");
        assert_eq!(json["target"], "start-menu-folder");

        let custom = serde_json::to_value(DirKey::Custom("data".into())).unwrap();
        assert_eq!(custom, serde_json::json!({ "custom": "data" }));
    }

    #[test]
    fn spec_round_trips_through_json() {
        let spec = InstallerSpec {
            product: product(),
            payload: vec![PayloadFile {
                source: "target/jarvis.exe".into(),
                target: DirKey::ProgramFiles,
                file_name: None,
                attributes: 0,
                executable: true,
            }],
            shortcuts: vec![Shortcut {
                name: "MyJarvis".into(),
                component: "main".into(),
                target: "jarvis.exe".into(),
                arguments: None,
                working_dir: None,
                icon: None,
            }],
            output: "dist/installer/myjarvis.msi".into(),
        };
        let text = serde_json::to_string(&spec).unwrap();
        let back: InstallerSpec = serde_json::from_str(&text).unwrap();
        back.validate().unwrap();
    }
}
