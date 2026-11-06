//! 审计设施集成测试：四类结果、脱敏、并发追加、轮转与查询过滤。

use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    thread,
};

use serde_json::json;
use wae_audit::{Actor, ActorKind, Auditor, Mode, Outcome, Query, now_ms};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wae-audit-{name}-{}-{}", process::id(), now_ms()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn auditor(dir: &Path, max_bytes: u64) -> Auditor {
    Auditor::builder("demo-app", "0.1.0")
        .dir(dir)
        .actor(Actor { kind: ActorKind::Agent, id: "agent-1".into(), source: "cli".into() })
        .host("host-1")
        .session("session-1")
        .max_bytes(max_bytes)
        .build()
}

fn read_lines(dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            lines.extend(fs::read_to_string(path).unwrap().lines().map(str::to_string));
        }
    }
    lines
}

#[test]
fn records_success_failure_rejected_and_dry_run() {
    let tmp = TempDir::new("four-outcomes");
    let auditor = auditor(tmp.path(), 8 * 1024 * 1024);

    auditor.call("scan", Mode::Execute).params(json!({ "root": "C:\\tmp" })).targets(3, "disk entries").succeed().unwrap();
    auditor.call("scan", Mode::Execute).fail("scan::io_error").unwrap();
    auditor.call("purge", Mode::Execute).reject("auth::denied").unwrap();
    auditor.call("purge", Mode::DryRun).targets(2, "candidates").succeed().unwrap();

    let entries = auditor.query(&Query::default()).unwrap();
    assert_eq!(entries.len(), 4);

    let success = entries.iter().find(|entry| entry.capability == "scan" && entry.outcome == Outcome::Success).unwrap();
    assert_eq!(success.mode, Mode::Execute);
    assert_eq!(success.schema, "wae.audit/v0");
    assert_eq!(success.capability, "scan");
    assert_eq!(success.app, "demo-app");
    assert_eq!(success.app_version, "0.1.0");
    assert_eq!(success.actor.kind, ActorKind::Agent);
    assert_eq!(success.actor.id, "agent-1");
    assert_eq!(success.actor.source, "cli");
    assert_eq!(success.host, "host-1");
    assert_eq!(success.session, "session-1");
    assert_eq!(success.params["root"], "C:\\tmp");
    assert_eq!(success.targets.count, 3);
    assert_eq!(success.targets.summary, "disk entries");
    assert_eq!(success.error_code, None);

    let failure = entries.iter().find(|entry| entry.outcome == Outcome::Failure).unwrap();
    assert_eq!(failure.error_code.as_deref(), Some("scan::io_error"));

    let rejected = entries.iter().find(|entry| entry.outcome == Outcome::Rejected).unwrap();
    assert_eq!(rejected.capability, "purge");
    assert_eq!(rejected.error_code.as_deref(), Some("auth::denied"));

    let dry_run = entries.iter().find(|entry| entry.mode == Mode::DryRun).unwrap();
    assert_eq!(dry_run.targets.count, 2);

    let lines = read_lines(tmp.path());
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().all(|line| serde_json::from_str::<wae_audit::Entry>(line).is_ok()));
}

#[test]
fn sensitive_values_never_reach_the_log() {
    let tmp = TempDir::new("sanitize");
    let auditor = auditor(tmp.path(), 8 * 1024 * 1024);

    auditor
        .call("login", Mode::Execute)
        .params(json!({
            "user": "alice",
            "token": "TOKENVALUE",
            "password": "PASSWORDVALUE",
            "apiKey": "APIKEYVALUE",
            "authorization": "Bearer BEARERVALUE",
            "privateKey": "-----BEGIN RSA PRIVATE KEY-----\nKEYMATERIAL\n-----END RSA PRIVATE KEY-----",
            "nested": { "cookie": "COOKIEVALUE", "keep": "visible" },
        }))
        .succeed()
        .unwrap();

    let raw = read_lines(tmp.path()).join("\n");
    for secret in ["TOKENVALUE", "PASSWORDVALUE", "APIKEYVALUE", "BEARERVALUE", "KEYMATERIAL", "COOKIEVALUE"] {
        assert!(!raw.contains(secret), "日志仍包含敏感值 {secret}");
    }
    assert!(raw.contains("<redacted>"));
    assert!(raw.contains("alice"));
    assert!(raw.contains("visible"));
}

#[test]
fn concurrent_appends_do_not_lose_lines() {
    let tmp = TempDir::new("concurrent");
    let auditor = Arc::new(auditor(tmp.path(), 8 * 1024 * 1024));

    let mut handles = Vec::new();
    for thread_id in 0..8u64 {
        let auditor = Arc::clone(&auditor);
        handles.push(thread::spawn(move || {
            for index in 0..100u64 {
                auditor.call("scan", Mode::Execute).params(json!({ "seq": thread_id * 100 + index })).succeed().unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(read_lines(tmp.path()).len(), 800);
    let entries = auditor.query(&Query::default()).unwrap();
    assert_eq!(entries.len(), 800);

    let mut sequences: Vec<u64> = entries.iter().map(|entry| entry.params["seq"].as_u64().unwrap()).collect();
    sequences.sort_unstable();
    sequences.dedup();
    assert_eq!(sequences.len(), 800);
}

#[test]
fn rotation_keeps_records_queryable_and_filters_work() {
    let tmp = TempDir::new("rotation");
    let auditor = auditor(tmp.path(), 512);

    for index in 0..20u64 {
        let capability = if index % 2 == 0 { "even" } else { "odd" };
        auditor.call(capability, Mode::Execute).params(json!({ "i": index })).succeed().unwrap();
    }

    let rotated = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| name.starts_with("demo-app.") && name.ends_with(".jsonl")))
        .count();
    assert!(rotated > 0, "达到阈值后应产生轮转文件");

    assert_eq!(auditor.query(&Query::default()).unwrap().len(), 20);

    let limited = auditor.query(&Query { limit: Some(5), ..Default::default() }).unwrap();
    assert_eq!(limited.len(), 5);

    let even = auditor.query(&Query { capability: Some("even".into()), ..Default::default() }).unwrap();
    assert_eq!(even.len(), 10);

    let agents = auditor.query(&Query { actor_kind: Some(ActorKind::Agent), ..Default::default() }).unwrap();
    assert_eq!(agents.len(), 20);
    let users = auditor.query(&Query { actor_kind: Some(ActorKind::User), ..Default::default() }).unwrap();
    assert!(users.is_empty());

    let now = now_ms();
    let in_range = auditor.query(&Query { from_ms: Some(now.saturating_sub(5_000)), to_ms: Some(now + 5_000), ..Default::default() }).unwrap();
    assert_eq!(in_range.len(), 20);
    let future = auditor.query(&Query { from_ms: Some(now + 60_000), ..Default::default() }).unwrap();
    assert!(future.is_empty());
}

#[test]
fn abandoned_calls_are_recorded_as_failure() {
    let tmp = TempDir::new("abandoned");
    let auditor = auditor(tmp.path(), 8 * 1024 * 1024);

    {
        let _unfinished = auditor.call("scan", Mode::Execute);
    }

    let entries = auditor.query(&Query::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, Outcome::Failure);
    assert_eq!(entries[0].error_code.as_deref(), Some("wae.audit::abandoned"));
}
