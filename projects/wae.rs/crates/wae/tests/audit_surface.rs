//! 门面审计入口：应用只补领域字段，公共字段由设施补齐。

use std::{fs, path::PathBuf};

use wae::audit::{Actor, ActorKind, Auditor, Mode, Outcome, Query};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wae-facade-audit-{name}-{}-{}", std::process::id(), wae::audit::now_ms()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn auditor(dir: &PathBuf) -> Auditor {
    Auditor::builder("facade-app", "0.1.0")
        .dir(dir)
        .actor(Actor { kind: ActorKind::Agent, id: "agent-1".into(), source: "cli".into() })
        .host("host-1")
        .session("session-1")
        .max_bytes(1024 * 1024)
        .build()
}

#[test]
fn facade_records_and_queries_audit_records() {
    let dir = temp_dir("records");
    let auditor = auditor(&dir);

    auditor.call("scan", Mode::Execute).params_json(r#"{"root":"C:\\tmp","token":"SECRETVALUE"}"#).targets(1, "scan roots").succeed().unwrap();

    let entries = auditor.query(&Query::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].capability, "scan");
    assert_eq!(entries[0].actor.kind, ActorKind::Agent);
    assert_eq!(entries[0].actor.id, "agent-1");
    assert_eq!(entries[0].mode, Mode::Execute);
    assert_eq!(entries[0].outcome, Outcome::Success);

    let json = auditor.query_json(&Query::default()).unwrap();
    assert!(json.contains("wae.audit/v0"));
    assert!(!json.contains("SECRETVALUE"));
    assert!(json.contains("<redacted>"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn napi_call_separates_refusals_from_failures() {
    let dir = temp_dir("napi-outcome");
    let auditor = auditor(&dir);

    let refused =
        wae::audit::napi_call(&auditor, "install-apply", Mode::Execute).run(|| Err(wae::Error::new("blocked").refused("dependency_blocked")));
    assert!(refused.is_err());
    let failed = wae::audit::napi_call(&auditor, "inspect", Mode::Execute).run(|| Err(wae::Error::new("i/o failure")));
    assert!(failed.is_err());

    let entries = auditor.query(&Query::default()).unwrap();
    let refusal = entries.iter().find(|entry| entry.capability == "install-apply").unwrap();
    assert_eq!(refusal.outcome, Outcome::Rejected);
    assert_eq!(refusal.error_code.as_deref(), Some("dependency_blocked"));
    let failure = entries.iter().find(|entry| entry.capability == "inspect").unwrap();
    assert_eq!(failure.outcome, Outcome::Failure);
    assert_eq!(failure.error_code.as_deref(), Some("wae.audit::error"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn napi_entry_builders_are_exposed_on_the_facade() {
    let dir = temp_dir("napi");
    let auditor = auditor(&dir);

    let _query: fn(&wae::CallContext, &Auditor) -> wae::Result<wae::JsValue> = wae::audit::query_from_napi;
    {
        let _pending = wae::audit::napi_call(&auditor, "scan", Mode::DryRun).targets(2, "candidates");
    }

    let entries = auditor.query(&Query::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].mode, Mode::DryRun);
    assert_eq!(entries[0].outcome, Outcome::Failure);
    assert_eq!(entries[0].error_code.as_deref(), Some("wae.audit::abandoned"));

    let _ = fs::remove_dir_all(&dir);
}
