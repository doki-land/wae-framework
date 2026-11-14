//! Platform-neutral N-API surface for {{nameTitle}}.
//!
//! Every export is typed: `#[function]` converts parameters through `FromJs` and
//! returns through `ToJs`, and `#[object]` defines the JS boundary shape of the
//! option structs. `wae build` renders one `src/native.ts` from the addon's
//! `__waeTypes` metadata, so this file carries no hand-written field extraction
//! and no JSON bridge.
//!
//! The crate contains no platform `cfg` branches and no platform string
//! literals; missing host capabilities arrive as structured upstream errors.
//! Audit is wrapped at every capability boundary through the shared
//! `wae.audit/v0` facility.

#![deny(missing_docs)]

use std::cell::RefCell;

use serde_json::Value;
use wae::{
    Json, Result, function, object, register_module,
    webview::{OpenOptions, WebViewError, open},
};
use {{nameSnake}}_core::{
    AuditFields, Capabilities, Capability, CapabilityState, Error as CoreError, InspectionOptions, InspectionReport, InstallOptions, InstallPlan,
    UninstallOptions, UninstallPlan, apply_install, apply_uninstall, capabilities as core_capabilities, capability_ids, inspect, plan_install,
    plan_uninstall,
};

mod audit;
mod ipc;

use audit::Summary;
use wae::audit::Mode;

/// Window title used when the caller does not pass one.
const APP_TITLE: &str = "{{nameTitle}}";

/// Environment variable that pins the GUI content directory.
const GUI_DIR_ENV: &str = "{{nameUpperSnake}}_GUI";

thread_local! {
    /// The single window this process owns, if any.
    static GUI: RefCell<Option<wae::webview::WebViewHost>> = const { RefCell::new(None) };
}

/// Options for directory inspection.
#[object]
#[derive(Debug, Clone, Default)]
struct InspectRequest {
    root: Option<String>,
    limit: Option<u64>,
}

/// Options shared by install planning and application.
#[object]
#[derive(Debug, Clone, Default)]
struct InstallRequest {
    no_shortcut: Option<bool>,
    reset_config: Option<bool>,
}

/// Options for uninstall planning and application.
#[object]
#[derive(Debug, Clone, Default)]
struct UninstallRequest {
    keep_config: Option<bool>,
}

/// Options for opening the GUI.
#[object]
#[derive(Debug, Clone, Default)]
struct GuiRequest {
    content_dir: Option<String>,
    title: Option<String>,
}

/// Caller identity used by the shared audit facility.
#[object]
#[derive(Debug, Clone, Default)]
struct ActorRequest {
    actor_kind: Option<String>,
    actor_id: Option<String>,
    actor_source: Option<String>,
    session: Option<String>,
    host: Option<String>,
}

/// Audit query filter.
#[object]
#[derive(Debug, Clone, Default)]
struct AuditRequest {
    kind: Option<String>,
    id: Option<String>,
    capability: Option<String>,
    from_ms: Option<u64>,
    to_ms: Option<u64>,
    limit: Option<u64>,
}

/// Result of a completed mutating capability.
#[object]
#[derive(Debug, Clone)]
struct OkResult {
    ok: bool,
}

/// Result of opening the GUI window.
#[object]
#[derive(Debug, Clone)]
struct OpenedWindow {
    backend: String,
}

/// Capability report with the upstream facade row the domain cannot know about.
///
/// The `wae` facade only compiles on targets that carry a WebView backend, so
/// `gui` exists wherever this crate builds. Missing runtime libraries still
/// surface from `gui-open` as `dependency_blocked`, because the Linux backend
/// resolves GTK at runtime rather than at build time.
pub(crate) fn capability_report() -> Capabilities {
    let mut report = core_capabilities();
    report.items.push(Capability::available(capability_ids::GUI));
    let note = "`gui` covers `openGui`, `pumpEvents`, `closeGui` and `isGuiOpen`, missing runtime dependencies surface as `dependency_blocked` from `gui-open`";
    report.notes.push(note.to_string());
    report
}

/// Map a domain error onto the N-API error contract.
pub(crate) fn core_error(error: CoreError) -> wae::Error {
    let payload = error.payload();
    let text = match serde_json::to_string(&payload) {
        Ok(json) => format!("wae.error:{json}"),
        Err(_) => "wae.error:{\"code\":\"message\",\"message\":\"error payload serialization failed\"}".to_string(),
    };
    // Capability errors are refusals, so the audit record uses `rejected` with the
    // capability state code instead of a generic execution failure.
    if matches!(&error, CoreError::Capability { .. }) { wae::Error::new(text).refused(error.code()) } else { wae::Error::new(text) }
}

/// Upstream host failure codes that mean a required GUI dependency is missing.
const GUI_DEPENDENCY_CODES: &[&str] = &["webview2_runtime_missing", "webkit_dependency_missing", "webkit_symbol_missing", "gtk_init_failed"];

fn webview_error(error: WebViewError) -> CoreError {
    match &error {
        WebViewError::Unsupported { capability, .. } => CoreError::capability(
            capability_ids::GUI,
            CapabilityState::UnsupportedPlatform,
            format!("upstream webview capability {capability:?} is unavailable on this target"),
        ),
        WebViewError::Host(failure) if GUI_DEPENDENCY_CODES.contains(&failure.code) => {
            CoreError::capability(capability_ids::GUI, CapabilityState::DependencyBlocked, error.to_string())
        }
        _ => CoreError::message(error.to_string()),
    }
}

fn resolve_gui_dir(requested: Option<String>) -> Result<String> {
    if let Some(dir) = requested
        && !dir.trim().is_empty()
    {
        return Ok(dir);
    }
    if let Ok(dir) = std::env::var(GUI_DIR_ENV)
        && !dir.trim().is_empty()
    {
        return Ok(dir);
    }
    let candidates = [std::path::PathBuf::from("packages/{{name}}/dist/gui"), std::path::PathBuf::from("dist/gui"), std::path::PathBuf::from("gui")];
    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return Ok(candidate.display().to_string());
        }
    }
    Err(wae::Error::new("gui content dir not found: pass `contentDir` or build `packages/{{name}}/dist/gui`"))
}

fn actor_kind(value: Option<&str>) -> wae::audit::ActorKind {
    match value {
        Some(value) if value.eq_ignore_ascii_case("agent") => wae::audit::ActorKind::Agent,
        _ => wae::audit::ActorKind::User,
    }
}

/// Native addon version.
#[function]
fn version() -> Result<String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

/// Capability report, including rows owned by the upstream facade.
#[function]
fn capabilities() -> Result<Json<Capabilities>> {
    audit::run(capability_ids::CAPABILITIES, Mode::DryRun, Value::Null, || {
        let report = capability_report();
        let summary = Summary::new(report.items.len() as u64, "capabilities");
        Ok((report, summary))
    })
    .map(Json)
    .map_err(core_error)
}

/// Inspect a directory tree.
#[function(js_name = "inspect")]
fn inspect_tree(request: Option<InspectRequest>) -> Result<Json<InspectionReport>> {
    let request = request.unwrap_or_default();
    let options = InspectionOptions { root: request.root, limit: request.limit.map(|value| value as usize) };
    let params = serde_json::json!({ "root": &options.root, "limit": options.limit });
    audit::run(capability_ids::INSPECT, Mode::Execute, params, move || {
        let report = inspect(options)?;
        let fields = AuditFields::for_report(&report);
        Ok((report, Summary::new(fields.target_count, fields.target_summary)))
    })
    .map(Json)
    .map_err(core_error)
}

/// Plan an install or upgrade without touching host state.
#[function(js_name = "installPlan")]
fn install_plan(request: Option<InstallRequest>) -> Result<Json<InstallPlan>> {
    let options = install_options(request);
    let params = serde_json::json!({ "noShortcut": options.no_shortcut, "resetConfig": options.reset_config });
    audit::run(capability_ids::INSTALL_PLAN, Mode::DryRun, params, move || {
        let plan = plan_install(&options)?;
        let fields = AuditFields::for_install_plan(&plan);
        Ok((plan, Summary::new(fields.target_count, fields.target_summary)))
    })
    .map(Json)
    .map_err(core_error)
}

/// Apply an install or upgrade.
#[function(js_name = "installApply")]
fn install_apply(request: Option<InstallRequest>) -> Result<OkResult> {
    let options = install_options(request);
    let params = serde_json::json!({ "noShortcut": options.no_shortcut, "resetConfig": options.reset_config });
    audit::run(capability_ids::INSTALL_APPLY, Mode::Execute, params, move || {
        apply_install(&options)?;
        Ok(((), Summary::new(0, "install applied")))
    })
    .map(|()| OkResult { ok: true })
    .map_err(core_error)
}

/// Plan an uninstall without touching host state.
#[function(js_name = "uninstallPlan")]
fn uninstall_plan(request: Option<UninstallRequest>) -> Result<Json<UninstallPlan>> {
    let options = uninstall_options(request);
    let params = serde_json::json!({ "keepConfig": options.keep_config });
    audit::run(capability_ids::UNINSTALL_PLAN, Mode::DryRun, params, move || {
        let plan = plan_uninstall(&options)?;
        let fields = AuditFields::for_uninstall_plan(&plan);
        Ok((plan, Summary::new(fields.target_count, fields.target_summary)))
    })
    .map(Json)
    .map_err(core_error)
}

/// Apply an uninstall.
#[function(js_name = "uninstallApply")]
fn uninstall_apply(request: Option<UninstallRequest>) -> Result<OkResult> {
    let options = uninstall_options(request);
    let params = serde_json::json!({ "keepConfig": options.keep_config });
    audit::run(capability_ids::UNINSTALL_APPLY, Mode::Execute, params, move || {
        apply_uninstall(&options)?;
        Ok(((), Summary::new(0, "uninstall applied")))
    })
    .map(|()| OkResult { ok: true })
    .map_err(core_error)
}

/// Open the GUI window.
#[function(js_name = "openGui")]
fn open_gui(request: Option<GuiRequest>) -> Result<OpenedWindow> {
    let request = request.unwrap_or_default();
    let content_dir = resolve_gui_dir(request.content_dir)?;
    let title = request.title.unwrap_or_else(|| APP_TITLE.to_string());
    let params = serde_json::json!({ "contentDir": &content_dir, "title": &title });
    audit::run(capability_ids::GUI_OPEN, Mode::Execute, params, || {
        let options = OpenOptions::from_content_dir(&content_dir, &title);
        let host = open(options, Box::new(ipc::HostIpc)).map_err(webview_error)?;
        let backend = host.backend_name().to_string();
        GUI.with(|slot| {
            *slot.borrow_mut() = Some(host);
        });
        Ok((backend, Summary::new(1, "gui window opened")))
    })
    .map(|backend| OpenedWindow { backend })
    .map_err(core_error)
}

/// Pump one GUI event batch.
#[function(js_name = "pumpEvents")]
fn pump_events() -> Result<bool> {
    audit::run(capability_ids::GUI_PUMP, Mode::Execute, Value::Null, || {
        let alive = GUI.with(|slot| slot.borrow_mut().as_mut().map(|host| host.pump()).unwrap_or(false));
        Ok((alive, Summary::new(1, "gui event pump")))
    })
    .map_err(core_error)
}

/// Request GUI close.
#[function(js_name = "closeGui")]
fn close_gui() -> Result<bool> {
    audit::run(capability_ids::GUI_CLOSE, Mode::Execute, Value::Null, || {
        let closed = GUI.with(|slot| {
            let mut slot = slot.borrow_mut();
            match slot.as_mut() {
                Some(host) => host.close().is_ok(),
                None => false,
            }
        });
        if closed {
            GUI.with(|slot| {
                slot.borrow_mut().take();
            });
        }
        Ok((closed, Summary::new(1, "gui window close")))
    })
    .map_err(core_error)
}

/// Whether the GUI window is still open.
#[function(js_name = "isGuiOpen")]
fn is_gui_open() -> Result<bool> {
    audit::run(capability_ids::GUI_STATUS, Mode::DryRun, Value::Null, || {
        let alive = GUI.with(|slot| slot.borrow().as_ref().map(|host| host.is_alive()).unwrap_or(false));
        Ok((alive, Summary::new(1, "gui status")))
    })
    .map_err(core_error)
}

/// Configure the shared audit facility for this process.
#[function(js_name = "configureAudit")]
fn configure_audit(request: Option<ActorRequest>) -> Result<OkResult> {
    let request = request.unwrap_or_default();
    audit::configure(
        wae::audit::Actor {
            kind: actor_kind(request.actor_kind.as_deref()),
            id: request.actor_id.unwrap_or_else(|| "unknown".to_string()),
            source: request.actor_source.unwrap_or_else(|| "local".to_string()),
        },
        request.session,
        request.host,
    );
    Ok(OkResult { ok: true })
}

/// Read audit entries for the current actor as a JSON array.
#[function(js_name = "auditQuery")]
fn audit_query(request: Option<AuditRequest>) -> Result<String> {
    let request = request.unwrap_or_default();
    let filter = wae::audit::Query {
        actor_kind: request.kind.as_deref().map(|value| actor_kind(Some(value))),
        actor_id: request.id,
        capability: request.capability,
        from_ms: request.from_ms,
        to_ms: request.to_ms,
        limit: request.limit.map(|value| value as usize),
    };
    let params = serde_json::json!({ "capability": &filter.capability, "limit": filter.limit });
    audit::run(capability_ids::AUDIT_QUERY, Mode::DryRun, params, || {
        let entries = audit::query(&filter).map_err(|error| CoreError::message(error.to_string()))?;
        let summary = Summary::new(entries.len() as u64, "audit query");
        Ok((entries, summary))
    })
    .map_err(core_error)
    .and_then(|entries| serde_json::to_string(&entries).map_err(|error| wae::Error::new(error.to_string())))
}

fn install_options(request: Option<InstallRequest>) -> InstallOptions {
    let request = request.unwrap_or_default();
    InstallOptions { no_shortcut: request.no_shortcut.unwrap_or(false), reset_config: request.reset_config.unwrap_or(false) }
}

fn uninstall_options(request: Option<UninstallRequest>) -> UninstallOptions {
    let request = request.unwrap_or_default();
    UninstallOptions { keep_config: request.keep_config.unwrap_or(true) }
}

register_module![
    version,
    capabilities,
    inspect_tree,
    install_plan,
    install_apply,
    uninstall_plan,
    uninstall_apply,
    open_gui,
    pump_events,
    close_gui,
    is_gui_open,
    configure_audit,
    audit_query
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_gui_runtime_dependencies_map_to_dependency_blocked() {
        for code in GUI_DEPENDENCY_CODES {
            let mapped = webview_error(WebViewError::host(code, "load_libraries", "missing dependency"));
            assert_eq!(mapped.code(), CapabilityState::DependencyBlocked.code());
            match mapped {
                CoreError::Capability { capability, .. } => assert_eq!(capability, capability_ids::GUI),
                other => panic!("unexpected domain error: {other}"),
            }
        }
    }

    #[test]
    fn other_host_failures_stay_plain_domain_errors() {
        let mapped = webview_error(WebViewError::host("create_window_failed", "create_window", "boom"));
        assert_eq!(mapped.code(), "message");
    }

    #[test]
    fn capability_report_lists_gui_and_reasons_for_unavailable_rows() {
        let report = capability_report();
        assert!(report.is_available(capability_ids::GUI));
        assert!(report.notes.iter().any(|note| note.contains("dependency_blocked")));
        for id in [capability_ids::HOST_INSTALL, capability_ids::HOST_UNINSTALL] {
            let item = report.get(id).unwrap_or_else(|| panic!("missing capability row: {id}"));
            assert!(!item.is_available());
            assert!(item.detail.as_deref().is_some_and(|detail| !detail.is_empty()));
        }
    }
}
