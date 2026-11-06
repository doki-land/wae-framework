//! Domain-side reservation for the shared `wae.audit/v0` record.
//!
//! The schema, actor, host, session, timestamps, duration, and sanitisation are
//! owned by `wae::audit`. This module reserves only the fields the domain must
//! supply at a capability boundary. Do not grow a second audit schema here.

use serde::Serialize;

use crate::{
    inspect::InspectionReport,
    install::{InstallPlan, UninstallPlan},
};

/// Capability ids shared by the domain, the N-API boundary, and skills.
pub mod capability_ids {
    /// Portable directory inspection.
    pub const INSPECT: &str = "inspect";
    /// Install-or-upgrade planning.
    pub const INSTALL_PLAN: &str = "install-plan";
    /// Install-or-upgrade execution.
    pub const INSTALL_APPLY: &str = "install-apply";
    /// Uninstall planning.
    pub const UNINSTALL_PLAN: &str = "uninstall-plan";
    /// Uninstall execution.
    pub const UNINSTALL_APPLY: &str = "uninstall-apply";
    /// Public install-or-upgrade capability reported to consumers.
    pub const HOST_INSTALL: &str = "host-install";
    /// Public uninstall capability reported to consumers.
    pub const HOST_UNINSTALL: &str = "host-uninstall";
    /// GUI host capability.
    pub const GUI: &str = "gui";
    /// GUI window open.
    pub const GUI_OPEN: &str = "gui-open";
    /// GUI event pump.
    pub const GUI_PUMP: &str = "gui-pump";
    /// GUI window close.
    pub const GUI_CLOSE: &str = "gui-close";
    /// GUI status probe.
    pub const GUI_STATUS: &str = "gui-status";
    /// Capability report.
    pub const CAPABILITIES: &str = "capabilities";
    /// Version report.
    pub const VERSION: &str = "version";
    /// Audit query.
    pub const AUDIT_QUERY: &str = "audit-query";
    /// Audit configuration.
    pub const AUDIT_CONFIGURE: &str = "audit-configure";
}

/// Fields the domain contributes to one audit record.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFields {
    /// Capability id.
    pub capability: String,
    /// Number of affected targets.
    pub target_count: u64,
    /// Human-readable target summary.
    pub target_summary: String,
}

impl AuditFields {
    /// Fields for a completed inspection.
    pub fn for_report(report: &InspectionReport) -> Self {
        Self {
            capability: capability_ids::INSPECT.to_string(),
            target_count: report.entries.len() as u64,
            target_summary: format!("{} entries under {} ({} bytes)", report.entry_count, report.root, report.total_bytes),
        }
    }

    /// Fields for an install plan.
    pub fn for_install_plan(plan: &InstallPlan) -> Self {
        Self {
            capability: capability_ids::INSTALL_PLAN.to_string(),
            target_count: plan.actions.len() as u64,
            target_summary: format!("{} action(s) for {} {} (upgrade={})", plan.actions.len(), plan.app, plan.version, plan.upgrade),
        }
    }

    /// Fields for an uninstall plan.
    pub fn for_uninstall_plan(plan: &UninstallPlan) -> Self {
        Self {
            capability: capability_ids::UNINSTALL_PLAN.to_string(),
            target_count: plan.actions.len() as u64,
            target_summary: format!("{} action(s) for {}", plan.actions.len(), plan.app),
        }
    }
}
