//! Wire snapshot for the audit row shape.
//!
//! `AuditEvent` is what the audit views serialize and what SDK consumers
//! read back. Its shape is a contract the lexicon snapshots do not cover:
//! which optional fields serialize as `null` and which are skipped
//! entirely (`grant_id`) is visible on the wire and easy to change by
//! accident with a serde attribute edit. The committed JSON is the review
//! surface — a shape change lands as a fixture diff, not a surprise.
//!
//! Regenerate with `MAIDAN_WRITE_SNAPSHOTS=1` when a change is intended.

use chrono::{DateTime, Utc};
use maidan_types::{AuditEvent, DelegationGrantId, MemberId, WorkspaceId};
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

fn ts() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("fixed unix seconds")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wire")
}

fn pretty(value: &Value) -> String {
    let mut s = serde_json::to_string_pretty(value).expect("pretty");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn assert_snapshot(name: &str, value: &Value) {
    let got = pretty(value);
    let path = fixtures_dir().join(name);
    if std::env::var("MAIDAN_WRITE_SNAPSHOTS").ok().as_deref() == Some("1") {
        std::fs::create_dir_all(fixtures_dir()).expect("mkdir");
        std::fs::write(&path, &got).expect("write snapshot");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got, expected,
        "wire snapshot drift in {name} — rerun with MAIDAN_WRITE_SNAPSHOTS=1 if intentional"
    );
}

/// Every field populated: the full row an audit view returns for a
/// delegated action inside a workspace.
fn a_full_audit_event() -> AuditEvent {
    AuditEvent {
        id: 42,
        occurred_at: ts(),
        actor_id: Some(MemberId(Uuid::from_u128(2))),
        subject_id: Some(MemberId(Uuid::from_u128(3))),
        grant_id: Some(DelegationGrantId(Uuid::from_u128(4))),
        action: "thread.transition".into(),
        target_kind: Some("thread".into()),
        target_id: Some(Uuid::from_u128(5)),
        metadata: json!({ "delivered_to": "github", "attempt": 2 }),
        workspace_id: Some(WorkspaceId(Uuid::from_u128(1))),
    }
}

/// An instance-level row: no actor, no target, no workspace. Pins which
/// fields serialize as `null` and which (`grant_id`) disappear.
fn a_minimal_audit_event() -> AuditEvent {
    AuditEvent {
        id: 43,
        occurred_at: ts(),
        actor_id: None,
        subject_id: None,
        grant_id: None,
        action: "instance.retention_swept".into(),
        target_kind: None,
        target_id: None,
        metadata: json!({}),
        workspace_id: None,
    }
}

#[test]
fn the_audit_row_shape_is_pinned() {
    let rows = vec![
        serde_json::to_value(a_full_audit_event()).expect("full row serializes"),
        serde_json::to_value(a_minimal_audit_event()).expect("minimal row serializes"),
    ];
    assert_snapshot("audit_events.json", &Value::Array(rows));
}
