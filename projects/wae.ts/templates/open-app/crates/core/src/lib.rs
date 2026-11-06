//! {{nameTitle}} domain core.
//!
//! This crate owns portable domain semantics and the three capability states
//! (`unsupported_platform`, `not_implemented`, `dependency_blocked`). Platform
//! identity is reported for diagnostics only. The N-API crate must not branch on
//! the target platform or hard-code platform messages.

#![deny(missing_docs)]

mod audit;
mod error;
mod inspect;
mod install;
pub mod platform;

pub use audit::{AuditFields, capability_ids};
pub use error::{CapabilityState, Error, ErrorPayload, Result, capability_code};
pub use inspect::{InspectionEntry, InspectionOptions, InspectionReport, inspect};
pub use install::{
    APP_ID, APP_TITLE, ActionKind, InstallOptions, InstallPlan, PlannedAction, UninstallOptions, UninstallPlan, apply_install, apply_uninstall,
    plan_install, plan_install_with, plan_uninstall, plan_uninstall_with,
};
pub use platform::{Capabilities, Capability, Platform, capabilities, platform_arch, platform_name};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_states_use_the_three_stable_codes() {
        let codes = [CapabilityState::UnsupportedPlatform, CapabilityState::NotImplemented, CapabilityState::DependencyBlocked];
        assert_eq!(codes.map(CapabilityState::code), ["unsupported_platform", "not_implemented", "dependency_blocked"]);
    }

    #[test]
    fn capability_report_marks_skeleton_install_as_not_implemented() {
        let report = capabilities();
        assert!(report.is_available(capability_ids::INSPECT));
        let install = report.get(capability_ids::HOST_INSTALL).unwrap();
        assert_eq!(install.state, Some(CapabilityState::NotImplemented));
    }

    #[test]
    fn error_payload_is_machine_readable() {
        let error = Error::capability(capability_ids::HOST_INSTALL, CapabilityState::DependencyBlocked, "missing host dependency");
        let payload = error.payload();
        assert_eq!(payload.code, "dependency_blocked");
        assert_eq!(payload.capability.as_deref(), Some("host-install"));
        assert!(payload.platform == platform_name());
    }
}
