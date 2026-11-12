//! Platform-neutral N-API surface for {{nameTitle}}.
//!
//! This crate forwards generic capabilities to the `wae` facade and contains no
//! platform `cfg` branches and no platform string literals. Missing host
//! capabilities arrive as structured upstream errors. Audit is wrapped at every
//! capability boundary through the shared `wae.audit/v0` facility.

#![deny(missing_docs)]

use std::{cell::RefCell, env, path::PathBuf};

use {{nameSnake}}_core::{
    AuditFields, Capabilities, Capability, CapabilityState, Error as CoreError, InspectionOptions, InstallOptions, UninstallOptions,
    apply_install, apply_uninstall, capabilities as core_capabilities, capability_ids, inspect, plan_install, plan_uninstall,
};
use serde_json::{Value, json};
use wae::{
    CallContext, Error, JsValue, Result, function, optional_string_prop, register_module,
    webview::{IpcHandler, IpcOutcome, IpcSink, OpenOptions, WebViewError, open},
};

mod audit;

use audit::Summary;
use wae::audit::Mode;

const APP_TITLE: &str = "{{nameTitle}}";
const GUI_DIR_ENV: &str = "{{nameUpperSnake}}_GUI";

thread_local! {
    static GUI: RefCell<Option<wae::webview::WebViewHost>> = const { RefCell::new(None) };
}

struct HostIpc;

impl IpcHandler for HostIpc {
    fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
        let Ok(request) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::ReplyToPage(r#"{"ok":false,"error":"invalid json"}"#.to_string());
        };
        let kind = request.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "close" => IpcOutcome::CloseWindow,
            "capabilities" => match audit::run(capability_ids::CAPABILITIES, Mode::Execute, Value::Null, || {
                Ok((capability_report(), Summary::new(1, "capabilities")))
            }) {
                Ok(report) => IpcOutcome::ReplyToPage(reply(json!({ "ok": true, "data": report }))),
                Err(error) => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": error.payload() }))),
            },
            "inspect" => {
                let options = InspectionOptions {
                    root: request.get("root").and_then(Value::as_str).map(str::to_string),
                    limit: request.get("limit").and_then(Value::as_u64).map(|value| value as usize),
                };
                let params = json!({ "root": &options.root, "limit": options.limit });
                match audit::run(capability_ids::INSPECT, Mode::Execute, params, move || {
                    let report = inspect(options)?;
                    let count = report.entry_count as u64;
                    Ok((report, Summary::new(count, "inspection")))
                }) {
                    Ok(report) => IpcOutcome::ReplyToPage(reply(json!({ "ok": true, "data": report }))),
                    Err(error) => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": error.payload() }))),
                }
            }
            "installPlan" => {
                let options = InstallOptions {
                    no_shortcut: request.get("noShortcut").and_then(Value::as_bool).unwrap_or(false),
                    reset_config: request.get("resetConfig").and_then(Value::as_bool).unwrap_or(false),
                };
                let params = json!({ "noShortcut": options.no_shortcut, "resetConfig": options.reset_config });
                match audit::run(capability_ids::INSTALL_PLAN, Mode::DryRun, params, move || {
                    let plan = plan_install(&options)?;
                    let fields = AuditFields::for_install_plan(&plan);
                    Ok((plan, Summary::new(fields.target_count, fields.target_summary)))
                }) {
                    Ok(plan) => IpcOutcome::ReplyToPage(reply(json!({ "ok": true, "data": plan }))),
                    Err(error) => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": error.payload() }))),
                }
            }
            "uninstallPlan" => {
                let options = UninstallOptions { keep_config: request.get("keepConfig").and_then(Value::as_bool).unwrap_or(true) };
                let params = json!({ "keepConfig": options.keep_config });
                match audit::run(capability_ids::UNINSTALL_PLAN, Mode::DryRun, params, move || {
                    let plan = plan_uninstall(&options)?;
                    let fields = AuditFields::for_uninstall_plan(&plan);
                    Ok((plan, Summary::new(fields.target_count, fields.target_summary)))
                }) {
                    Ok(plan) => IpcOutcome::ReplyToPage(reply(json!({ "ok": true, "data": plan }))),
                    Err(error) => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": error.payload() }))),
                }
            }
            other => IpcOutcome::ReplyToPage(reply(json!({ "ok": false, "error": format!("unknown message: {other}") }))),
        }
    }
}

fn reply(value: Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| r#"{"ok":false,"error":"serialization failed"}"#.to_string())
}

fn core_error(error: CoreError) -> Error {
    let payload = error.payload();
    let text = match serde_json::to_string(&payload) {
        Ok(json) => format!("wae.error:{json}"),
        Err(_) => "wae.error:{\"code\":\"message\",\"message\":\"error payload serialization failed\"}".to_string(),
    };
    // Capability errors are refusals, so the audit record uses `rejected` with the
    // capability state code instead of a generic execution failure.
    if matches!(&error, CoreError::Capability { .. }) { Error::new(text).refused(error.code()) } else { Error::new(text) }
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

/// Capability report with the upstream facade row the domain cannot know about.
///
/// The `wae` facade only compiles on targets that carry a WebView backend, so `gui`
/// exists wherever this crate builds. Missing runtime libraries still surface from
/// `gui-open` as `dependency_blocked`, because the Linux backend resolves GTK at
/// runtime rather than at build time.
fn capability_report() -> Capabilities {
    let mut report = core_capabilities();
    report.items.push(Capability::available(capability_ids::GUI));
    let note = "`gui` covers `openGui`, `pumpEvents`, `closeGui` and `isGuiOpen`, missing runtime dependencies surface as `dependency_blocked` from `gui-open`";
    report.notes.push(note.to_string());
    report
}

fn json_to_js(env: wae::Env, value: Value) -> Result<JsValue> {
    match value {
        Value::Null => env.null(),
        Value::Bool(value) => env.boolean(value),
        Value::Number(number) => match number.as_i64() {
            Some(value) => env.int64(value),
            None => env.double(number.as_f64().unwrap_or(0.0)),
        },
        Value::String(text) => env.string(&text),
        Value::Array(items) => {
            let array = env.array_with_length(items.len())?;
            for (index, item) in items.into_iter().enumerate() {
                array.set_element(index as u32, json_to_js(env, item)?)?;
            }
            Ok(array)
        }
        Value::Object(map) => {
            let object = env.object()?;
            for (key, item) in map {
                object.set_named(&key, json_to_js(env, item)?)?;
            }
            Ok(object)
        }
    }
}

fn value_to_js<T: serde::Serialize>(ctx: &CallContext, value: &T) -> Result<JsValue> {
    let json = serde_json::to_value(value).map_err(|error| Error::new(format!("serialize failed: {error}")))?;
    json_to_js(ctx.env(), json)
}

fn object_arg(ctx: &CallContext) -> Result<Option<JsValue>> {
    let Some(arg) = ctx.args().first().copied()
    else {
        return Ok(None);
    };
    if arg.is_undefined()? {
        return Ok(None);
    }
    Ok(Some(arg))
}

fn optional_bool(obj: Option<JsValue>, name: &str) -> Result<bool> {
    let Some(obj) = obj
    else {
        return Ok(false);
    };
    let value = obj.get_named(name)?;
    if value.is_undefined()? {
        return Ok(false);
    }
    value.as_bool()
}

fn optional_usize(obj: Option<JsValue>, name: &str) -> Result<Option<usize>> {
    let Some(obj) = obj
    else {
        return Ok(None);
    };
    let value = obj.get_named(name)?;
    if value.is_undefined()? {
        return Ok(None);
    }
    Ok(Some(value.as_i64()?.max(0) as usize))
}

fn inspect_options(ctx: &CallContext) -> Result<InspectionOptions> {
    let arg = object_arg(ctx)?;
    Ok(InspectionOptions { root: arg.and_then(|obj| optional_string_prop(obj, "root").ok().flatten()), limit: optional_usize(arg, "limit")? })
}

fn install_options(ctx: &CallContext) -> Result<InstallOptions> {
    let arg = object_arg(ctx)?;
    Ok(InstallOptions { no_shortcut: optional_bool(arg, "noShortcut")?, reset_config: optional_bool(arg, "resetConfig")? })
}

fn uninstall_options(ctx: &CallContext) -> Result<UninstallOptions> {
    let arg = object_arg(ctx)?;
    Ok(UninstallOptions { keep_config: optional_bool(arg, "keepConfig")? })
}

fn resolve_gui_dir(requested: Option<String>) -> Result<String> {
    if let Some(dir) = requested
        && !dir.trim().is_empty()
    {
        return Ok(dir);
    }
    if let Ok(dir) = env::var(GUI_DIR_ENV)
        && !dir.trim().is_empty()
    {
        return Ok(dir);
    }
    let candidates = [PathBuf::from("packages/{{name}}/dist/gui"), PathBuf::from("dist/gui"), PathBuf::from("gui")];
    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return Ok(candidate.display().to_string());
        }
    }
    Err(Error::new("gui content dir not found: pass `contentDir` or build `packages/{{name}}/dist/gui`"))
}

fn actor_kind(value: Option<&str>) -> wae::audit::ActorKind {
    match value {
        Some(value) if value.eq_ignore_ascii_case("agent") => wae::audit::ActorKind::Agent,
        _ => wae::audit::ActorKind::User,
    }
}

fn query_from_arg(arg: Option<JsValue>) -> Result<wae::audit::Query> {
    let mut query = wae::audit::Query::default();
    let Some(arg) = arg
    else {
        return Ok(query);
    };
    query.actor_kind = optional_string_prop(arg, "kind")?.map(|value| actor_kind(Some(value.as_str())));
    query.actor_id = optional_string_prop(arg, "id")?;
    query.capability = optional_string_prop(arg, "capability")?;
    query.from_ms = optional_i64(arg, "fromMs")?.map(|value| value.max(0) as u64);
    query.to_ms = optional_i64(arg, "toMs")?.map(|value| value.max(0) as u64);
    query.limit = optional_i64(arg, "limit")?.map(|value| value.max(0) as usize);
    Ok(query)
}

fn optional_i64(obj: JsValue, name: &str) -> Result<Option<i64>> {
    let value = obj.get_named(name)?;
    if value.is_undefined()? {
        return Ok(None);
    }
    Ok(Some(value.as_i64()?))
}

#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::VERSION, Mode::DryRun, Value::Null, || {
        Ok((env!("CARGO_PKG_VERSION").to_string(), Summary::new(0, "version")))
    });
    match result {
        Ok(version) => ctx.env().string(&version),
        Err(error) => Err(core_error(error)),
    }
}

#[function]
fn capabilities(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::CAPABILITIES, Mode::DryRun, Value::Null, || {
        let report = capability_report();
        let summary = Summary::new(report.items.len() as u64, "capabilities");
        Ok((report, summary))
    });
    match result {
        Ok(report) => value_to_js(&ctx, &report),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "inspect")]
fn inspect_tree(ctx: CallContext) -> Result<JsValue> {
    let options = inspect_options(&ctx)?;
    let params = json!({ "root": &options.root, "limit": options.limit });
    let result = audit::run(capability_ids::INSPECT, Mode::Execute, params, move || {
        let report = inspect(options)?;
        let fields = AuditFields::for_report(&report);
        let summary = Summary::new(fields.target_count, fields.target_summary);
        Ok((report, summary))
    });
    match result {
        Ok(report) => value_to_js(&ctx, &report),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "installPlan")]
fn install_plan(ctx: CallContext) -> Result<JsValue> {
    let options = install_options(&ctx)?;
    let params = json!({ "noShortcut": options.no_shortcut, "resetConfig": options.reset_config });
    let result = audit::run(capability_ids::INSTALL_PLAN, Mode::DryRun, params, move || {
        let plan = plan_install(&options)?;
        let fields = AuditFields::for_install_plan(&plan);
        let summary = Summary::new(fields.target_count, fields.target_summary);
        Ok((plan, summary))
    });
    match result {
        Ok(plan) => value_to_js(&ctx, &plan),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "installApply")]
fn install_apply(ctx: CallContext) -> Result<JsValue> {
    let options = install_options(&ctx)?;
    let params = json!({ "noShortcut": options.no_shortcut, "resetConfig": options.reset_config });
    let result = audit::run(capability_ids::INSTALL_APPLY, Mode::Execute, params, move || {
        apply_install(&options)?;
        Ok(((), Summary::new(0, "install applied")))
    });
    match result {
        Ok(()) => {
            let object = ctx.env().object()?;
            object.set_named("ok", ctx.env().boolean(true)?)?;
            Ok(object)
        }
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "uninstallPlan")]
fn uninstall_plan(ctx: CallContext) -> Result<JsValue> {
    let options = uninstall_options(&ctx)?;
    let params = json!({ "keepConfig": options.keep_config });
    let result = audit::run(capability_ids::UNINSTALL_PLAN, Mode::DryRun, params, move || {
        let plan = plan_uninstall(&options)?;
        let fields = AuditFields::for_uninstall_plan(&plan);
        let summary = Summary::new(fields.target_count, fields.target_summary);
        Ok((plan, summary))
    });
    match result {
        Ok(plan) => value_to_js(&ctx, &plan),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "uninstallApply")]
fn uninstall_apply(ctx: CallContext) -> Result<JsValue> {
    let options = uninstall_options(&ctx)?;
    let params = json!({ "keepConfig": options.keep_config });
    let result = audit::run(capability_ids::UNINSTALL_APPLY, Mode::Execute, params, move || {
        apply_uninstall(&options)?;
        Ok(((), Summary::new(0, "uninstall applied")))
    });
    match result {
        Ok(()) => {
            let object = ctx.env().object()?;
            object.set_named("ok", ctx.env().boolean(true)?)?;
            Ok(object)
        }
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "openGui")]
fn open_gui(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let content_dir = resolve_gui_dir(arg.and_then(|obj| optional_string_prop(obj, "contentDir").ok().flatten()))?;
    let title = arg.and_then(|obj| optional_string_prop(obj, "title").ok().flatten()).unwrap_or_else(|| APP_TITLE.to_string());
    let params = json!({ "contentDir": &content_dir, "title": &title });

    let result = audit::run(capability_ids::GUI_OPEN, Mode::Execute, params, || {
        let options = OpenOptions::from_content_dir(&content_dir, &title);
        let host = open(options, Box::new(HostIpc)).map_err(webview_error)?;
        let backend = host.backend_name().to_string();
        GUI.with(|slot| {
            *slot.borrow_mut() = Some(host);
        });
        Ok((backend, Summary::new(1, "gui window opened")))
    });
    match result {
        Ok(backend) => {
            let object = ctx.env().object()?;
            object.set_named("backend", ctx.env().string(&backend)?)?;
            Ok(object)
        }
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "pumpEvents")]
fn pump_events(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::GUI_PUMP, Mode::Execute, Value::Null, || {
        let alive = GUI.with(|slot| slot.borrow_mut().as_mut().map(|host| host.pump()).unwrap_or(false));
        Ok((alive, Summary::new(1, "gui event pump")))
    });
    match result {
        Ok(alive) => ctx.env().boolean(alive),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "closeGui")]
fn close_gui(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::GUI_CLOSE, Mode::Execute, Value::Null, || {
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
    });
    match result {
        Ok(closed) => ctx.env().boolean(closed),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "isGuiOpen")]
fn is_gui_open(ctx: CallContext) -> Result<JsValue> {
    let result = audit::run(capability_ids::GUI_STATUS, Mode::DryRun, Value::Null, || {
        let alive = GUI.with(|slot| slot.borrow().as_ref().map(|host| host.is_alive()).unwrap_or(false));
        Ok((alive, Summary::new(1, "gui status")))
    });
    match result {
        Ok(alive) => ctx.env().boolean(alive),
        Err(error) => Err(core_error(error)),
    }
}

#[function(js_name = "configureAudit")]
fn configure_audit(ctx: CallContext) -> Result<JsValue> {
    let arg = object_arg(&ctx)?;
    let actor = wae::audit::Actor {
        kind: actor_kind(arg.and_then(|obj| optional_string_prop(obj, "actorKind").ok().flatten()).as_deref()),
        id: arg.and_then(|obj| optional_string_prop(obj, "actorId").ok().flatten()).unwrap_or_else(|| "unknown".to_string()),
        source: arg.and_then(|obj| optional_string_prop(obj, "actorSource").ok().flatten()).unwrap_or_else(|| "local".to_string()),
    };
    let session = arg.and_then(|obj| optional_string_prop(obj, "session").ok().flatten());
    let host = arg.and_then(|obj| optional_string_prop(obj, "host").ok().flatten());
    audit::configure(actor, session, host);
    let object = ctx.env().object()?;
    object.set_named("ok", ctx.env().boolean(true)?)?;
    Ok(object)
}

#[function(js_name = "auditQuery")]
fn audit_query(ctx: CallContext) -> Result<JsValue> {
    let filter = query_from_arg(ctx.args().first().copied())?;
    let params = json!({ "capability": filter.capability, "limit": filter.limit });
    let result = audit::run(capability_ids::AUDIT_QUERY, Mode::DryRun, params, || {
        let entries = audit::query(&filter).map_err(|error| CoreError::message(error.to_string()))?;
        let summary = Summary::new(entries.len() as u64, "audit query");
        Ok((entries, summary))
    });
    match result {
        Ok(entries) => {
            let json = serde_json::to_string(&entries).map_err(|error| Error::new(error.to_string()))?;
            ctx.env().string(&json)
        }
        Err(error) => Err(core_error(error)),
    }
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
