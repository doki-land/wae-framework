//! GUI IPC bridge for the window host.
//!
//! The page talks JSON; every request maps onto a domain capability and keeps the
//! same audit attribution as the N-API exports. Replies are `{ ok, data }` or
//! `{ ok: false, error }`, so the page never parses prose.

use serde_json::{Value, json};
use wae::{
    audit::Mode,
    webview::{IpcHandler, IpcOutcome, IpcSink},
};
use {{nameSnake}}_core::{
    AuditFields, Error as CoreError, InspectionOptions, InstallOptions, UninstallOptions, capability_ids, inspect, plan_install, plan_uninstall,
};

use crate::{audit, capability_report};
use audit::Summary;

/// Handler for one window's IPC channel.
pub(crate) struct HostIpc;

impl IpcHandler for HostIpc {
    fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
        let Ok(request) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::ReplyToPage(r#"{"ok":false,"error":"invalid json"}"#.to_string());
        };
        let kind = request.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "close" => IpcOutcome::CloseWindow,
            "capabilities" => {
                let report = audit::run(capability_ids::CAPABILITIES, Mode::Execute, Value::Null, || {
                    Ok((capability_report(), Summary::new(1, "capabilities")))
                });
                reply(report.map(|report| json!(report)))
            }
            "inspect" => {
                let options = InspectionOptions {
                    root: request.get("root").and_then(Value::as_str).map(str::to_string),
                    limit: request.get("limit").and_then(Value::as_u64).map(|value| value as usize),
                };
                let params = json!({ "root": &options.root, "limit": options.limit });
                let report = audit::run(capability_ids::INSPECT, Mode::Execute, params, move || {
                    let report = inspect(options)?;
                    let fields = AuditFields::for_report(&report);
                    Ok((report, Summary::new(fields.target_count, fields.target_summary)))
                });
                reply(report.map(|report| json!(report)))
            }
            "installPlan" => {
                let options = InstallOptions {
                    no_shortcut: request.get("noShortcut").and_then(Value::as_bool).unwrap_or(false),
                    reset_config: request.get("resetConfig").and_then(Value::as_bool).unwrap_or(false),
                };
                let params = json!({ "noShortcut": options.no_shortcut, "resetConfig": options.reset_config });
                let plan = audit::run(capability_ids::INSTALL_PLAN, Mode::DryRun, params, move || {
                    let plan = plan_install(&options)?;
                    let fields = AuditFields::for_install_plan(&plan);
                    Ok((plan, Summary::new(fields.target_count, fields.target_summary)))
                });
                reply(plan.map(|plan| json!(plan)))
            }
            "uninstallPlan" => {
                let options = UninstallOptions { keep_config: request.get("keepConfig").and_then(Value::as_bool).unwrap_or(true) };
                let params = json!({ "keepConfig": options.keep_config });
                let plan = audit::run(capability_ids::UNINSTALL_PLAN, Mode::DryRun, params, move || {
                    let plan = plan_uninstall(&options)?;
                    let fields = AuditFields::for_uninstall_plan(&plan);
                    Ok((plan, Summary::new(fields.target_count, fields.target_summary)))
                });
                reply(plan.map(|plan| json!(plan)))
            }
            other => IpcOutcome::ReplyToPage(reply_err(&format!("unknown message: {other}"))),
        }
    }
}

fn reply(result: Result<Value, CoreError>) -> IpcOutcome {
    match result {
        Ok(data) => IpcOutcome::ReplyToPage(serialize(&json!({ "ok": true, "data": data }))),
        Err(error) => IpcOutcome::ReplyToPage(serialize(&json!({ "ok": false, "error": error.payload() }))),
    }
}

fn reply_err(message: &str) -> String {
    serialize(&json!({ "ok": false, "error": message }))
}

fn serialize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| r#"{"ok":false,"error":"serialization failed"}"#.to_string())
}
