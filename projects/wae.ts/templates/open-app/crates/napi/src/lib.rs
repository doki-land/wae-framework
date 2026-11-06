//! Platform-neutral N-API surface for {{nameTitle}}.
//!
//! This crate forwards generic capabilities to the `wae` facade and contains no
//! platform `cfg` branches and no platform string literals. Missing host
//! capabilities arrive as structured upstream errors. Audit is wrapped at every
//! capability boundary through the shared `wae.audit/v0` facility.

#![deny(missing_docs)]

use std::{cell::RefCell, env, path::PathBuf};

use {{nameSnake}}_core::{
    AuditFields, CapabilityState, Error as CoreError, InspectionOptions, InstallOptions, UninstallOptions, apply_install, apply_uninstall,
    capabilities as core_capabilities, capability_ids, inspect, plan_install, plan_uninstall,
};
use serde_json::{Value, json};
use wae::{
    CallContext, Error, JsValue, Result, function, optional_string_prop, register_module,
    webview::{IpcHandler, IpcOutcome, OpenOptions, WebViewError, open},
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
    fn on_ipc(&mut self, message: &str) -> IpcOutcome {
        let Ok(request) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::ReplyToPage(r#"{"ok":false,"error":"invalid json"}"#.to_string());
        };
        let kind = request.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "close" => IpcOutcome::CloseWindow,
            "capabilities" => match audit::run(capability_ids::CAPABILITIES, Mode::Execute, Value::Null, || {
                Ok((core_capabilities(), Summary::new(1, "capabilities")))
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
    match serde_json::to_string(&payload) {
        Ok(json) => Error::new(format!("wae.error:{json}")),
        Err(_) => Error::new("wae.error:{\"code\":\"message\",\"message\":\"error payload serialization failed\"}"),
    }
}

fn webview_error(error: WebViewError) -> CoreError {
    match error {
        WebViewError::Unsupported { capability, .. } => CoreError::capability(
            capability_ids::GUI,
            CapabilityState::UnsupportedPlatform,
            format!("upstream webview capability {capability:?} is unavailable on this target"),
        ),
        other => CoreError::message(other.to_string()),
    }
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
        let report = core_capabilities();
        let summary = Summary::new(report.items.len() as u64, "domain capabilities");
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
