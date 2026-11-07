//! Platform identity and the domain capability report.
//!
//! Platform identity exists for diagnostics only. Capability availability must
//! come from a domain backend or the upstream `wae` facade, never from an OS
//! comparison in application code.

use serde::Serialize;

use crate::{audit::capability_ids, error::CapabilityState};

/// Runtime platform identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows.
    Windows,
    /// macOS.
    Macos,
    /// Linux.
    Linux,
    /// Any other target.
    Other,
}

impl Platform {
    /// Detect the current target. Diagnostics only.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        }
        else if cfg!(target_os = "macos") {
            Self::Macos
        }
        else if cfg!(target_os = "linux") {
            Self::Linux
        }
        else {
            Self::Other
        }
    }

    /// Stable display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Other => "other",
        }
    }
}

/// Current platform display name.
pub fn platform_name() -> &'static str {
    Platform::current().name()
}

/// Current CPU architecture.
pub fn platform_arch() -> &'static str {
    std::env::consts::ARCH
}

/// One capability and its availability.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    /// Stable capability id.
    pub id: String,
    /// `null` means available.
    pub state: Option<CapabilityState>,
    /// Optional explanation.
    pub detail: Option<String>,
}

impl Capability {
    /// An available capability.
    pub fn available(id: &str) -> Self {
        Self { id: id.to_string(), state: None, detail: None }
    }

    /// An unavailable capability.
    pub fn unavailable(id: &str, state: CapabilityState, detail: impl Into<String>) -> Self {
        Self { id: id.to_string(), state: Some(state), detail: Some(detail.into()) }
    }

    /// Whether the capability can be called.
    pub const fn is_available(&self) -> bool {
        self.state.is_none()
    }
}

/// Domain capability report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Runtime platform identity.
    pub platform: String,
    /// Runtime CPU architecture.
    pub arch: String,
    /// Domain capabilities. The N-API boundary overlays capabilities owned by
    /// the upstream `wae` facade, such as `gui`.
    pub items: Vec<Capability>,
    /// Boundary notes that are not capabilities.
    pub notes: Vec<String>,
}

impl Capabilities {
    /// Look up one capability by id.
    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Whether one capability is available.
    pub fn is_available(&self, id: &str) -> bool {
        self.get(id).is_some_and(Capability::is_available)
    }
}

/// Report the domain capabilities of this skeleton.
pub fn capabilities() -> Capabilities {
    Capabilities {
        platform: platform_name().to_string(),
        arch: platform_arch().to_string(),
        items: vec![
            Capability::available(capability_ids::INSPECT),
            Capability::unavailable(
                capability_ids::HOST_INSTALL,
                CapabilityState::NotImplemented,
                "install planning is implemented, platform shortcut and config writes belong to the application domain",
            ),
            Capability::unavailable(
                capability_ids::HOST_UNINSTALL,
                CapabilityState::NotImplemented,
                "uninstall planning is implemented, platform removal belongs to the application domain",
            ),
        ],
        notes: Vec::new(),
    }
}
